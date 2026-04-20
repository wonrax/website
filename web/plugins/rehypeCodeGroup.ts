import type { Element, Root, RootContent, Text } from "hast";
import {
  CODE_GROUP_COMPONENT_NAME,
  CODE_GROUP_TITLE_ATTRIBUTE,
} from "./shared";

type MdxFlowNode = RootContent & {
  type: "mdxJsxFlowElement";
  name: string | null;
  children: RootContent[];
};

function isElementNode(node: RootContent): node is Element {
  return node.type === "element";
}

function isMdxFlowElement(node: RootContent): node is MdxFlowNode {
  return node.type === "mdxJsxFlowElement";
}

function isPrettyCodeFigure(node: RootContent): node is Element {
  return (
    isElementNode(node) &&
    node.tagName === "figure" &&
    node.properties?.["data-rehype-pretty-code-figure"] === ""
  );
}

function getTextContent(node: RootContent): string {
  if (node.type === "text") {
    return (node as Text).value;
  }

  if (!("children" in node) || !Array.isArray(node.children)) {
    return "";
  }

  return node.children.map(getTextContent).join("");
}

function normalizeCodeGroupFigure(node: Element): void {
  const [titleNode, ...restChildren] = node.children;

  if (
    titleNode != null &&
    isElementNode(titleNode) &&
    titleNode.properties?.className != null &&
    Array.isArray(titleNode.properties.className) &&
    titleNode.properties.className.includes("code-block-title")
  ) {
    node.properties = {
      ...node.properties,
      [CODE_GROUP_TITLE_ATTRIBUTE]: getTextContent(titleNode).split("/").at(-1),
    };
    node.children = restChildren;
    return;
  }

  node.properties = {
    ...node.properties,
    [CODE_GROUP_TITLE_ATTRIBUTE]: "",
  };
}

export default function rehypeCodeGroup() {
  return (tree: Root) => {
    for (const node of tree.children) {
      if (!isMdxFlowElement(node) || node.name !== CODE_GROUP_COMPONENT_NAME) {
        continue;
      }

      for (const child of node.children) {
        if (!isPrettyCodeFigure(child)) {
          continue;
        }

        normalizeCodeGroupFigure(child);
      }
    }
  };
}
