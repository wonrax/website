import type { Element, Root, RootContent } from "hast";
import { h } from "hastscript";
import {
  CODE_GROUP_COMPONENT_NAME,
  FEATURE_TYPE_ATTRIBUTE,
  getStringAttribute,
  isCustomImageNode,
} from "./shared";

type MdxFlowNode = RootContent & {
  type: "mdxJsxFlowElement";
  name: string | null;
  attributes: { type: string }[];
  children: RootContent[];
};

type NodeClassification =
  | {
      kind: "feature";
      className: string;
      wrapperTag: "div" | "figure";
    }
  | {
      kind: "reading";
      wrappedInFigure: boolean;
    };

function isElementNode(node: RootContent): node is Element {
  return node.type === "element";
}

function isMdxFlowElement(node: RootContent): node is MdxFlowNode {
  return node.type === "mdxJsxFlowElement";
}

function getFeatureWrapperClass(node: RootContent): string | undefined {
  if (isElementNode(node)) {
    if (node.tagName === "table") {
      return "feature-table";
    }

    if (node.tagName === "aside") {
      return "feature-aside";
    }

    if (node.tagName === "warning" || node.tagName === "note") {
      return "feature-callout";
    }

    if (
      node.tagName === "figure" &&
      node.properties?.["data-rehype-pretty-code-figure"] === ""
    ) {
      return "feature-code";
    }
  }

  if (isMdxFlowElement(node) && node.name === CODE_GROUP_COMPONENT_NAME) {
    return "feature-code";
  }

  return undefined;
}

function findCustomImageNode(node: RootContent): MdxFlowNode | undefined {
  if (isCustomImageNode(node)) {
    return node as MdxFlowNode;
  }

  if (!("children" in node) || !Array.isArray(node.children)) {
    return undefined;
  }

  return node.children.find((child): child is MdxFlowNode =>
    isCustomImageNode(child)
  );
}

function classifyNode(node: RootContent): NodeClassification {
  const featureWrapperClass = getFeatureWrapperClass(node);
  if (featureWrapperClass != null) {
    return {
      kind: "feature",
      className: featureWrapperClass,
      wrapperTag: "div",
    };
  }

  const imgNode = findCustomImageNode(node);
  if (imgNode == null) {
    return {
      kind: "reading",
      wrappedInFigure: false,
    };
  }

  const featureType = getStringAttribute(imgNode, FEATURE_TYPE_ATTRIBUTE);
  if (featureType == null) {
    return {
      kind: "reading",
      wrappedInFigure: true,
    };
  }

  return {
    kind: "feature",
    className: `feature feature-${featureType}`,
    wrapperTag: "figure",
  };
}

function wrapReadingNode(
  node: RootContent,
  wrappedInFigure: boolean
): RootContent {
  if (!wrappedInFigure) {
    return node;
  }

  return h("figure", [node]);
}

function wrapFeatureNode(
  node: RootContent,
  wrapperTag: "div" | "figure",
  className: string
): RootContent {
  return h(wrapperTag, { class: className.split(" ") }, [node]);
}

export default function rehypeBlogPost() {
  return (tree: Root) => {
    let wrapQueue: RootContent[] = [];
    const finalChildren: RootContent[] = [];

    function flushWrapper(): void {
      if (wrapQueue.length > 0) {
        finalChildren.push(
          h("div", { class: "reading-line-width" }, wrapQueue)
        );
        wrapQueue = [];
      }
    }

    for (const node of tree.children) {
      if (!isElementNode(node) && !isMdxFlowElement(node)) {
        finalChildren.push(node);
        continue;
      }

      const classification = classifyNode(node);
      if (classification.kind === "reading") {
        wrapQueue.push(wrapReadingNode(node, classification.wrappedInFigure));
        continue;
      }

      flushWrapper();
      finalChildren.push(
        wrapFeatureNode(
          node,
          classification.wrapperTag,
          classification.className
        )
      );
    }

    flushWrapper();
    tree.children = finalChildren;
  };
}
