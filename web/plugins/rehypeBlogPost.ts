import type { Root, RootContent } from "hast";
import { h } from "hastscript";
import type { MdxJsxFlowElement, MdxJsxAttribute } from "mdast-util-mdx";

// add extra custom modification like feature image etc.
export default function rehypeBlogPost() {
  return (tree: Root) => {
    // The queue that holds the normal elements that will be wrapped in a
    // MaxWidth component, whose width is reading line length. When a feature
    // image is encountered, the queue is flushed and the image is added
    // without the MaxWidth wrapper. This is inspired by react.dev
    let wrapQueue: RootContent[] = [];

    // The final children that will be set back to the tree
    const finalChildren: RootContent[] = [];

    function flushWrapper(): void {
      if (wrapQueue.length > 0) {
        finalChildren.push(
          h("div", { class: "reading-line-width" }, wrapQueue)
        );
        wrapQueue = [];
      }
    }

    for (let index = 0; index < tree.children.length; index++) {
      const node = tree.children[index];

      if (node.type !== "element" && node.type !== "mdxJsxFlowElement") {
        finalChildren.push(node);
        continue;
      }

      if (node.type === "element") {
        // table is a special case and is feature by default
        if (node.tagName === "table") {
          flushWrapper();
          finalChildren.push(h("div", { class: "feature-table" }, node));
          continue;
        }

        // aside is a special case and is feature by default
        if (node.tagName === "aside") {
          flushWrapper();
          finalChildren.push(h("div", { class: "feature-aside" }, node));
          continue;
        }

        if (["warning", "note"].includes(node.tagName)) {
          flushWrapper();
          finalChildren.push(h("div", { class: "feature-callout" }, node));
          continue;
        }

        // code block is a special case and is feature by default
        if (
          node.tagName === "figure" &&
          node.properties?.["data-rehype-pretty-code-figure"] === ""
        ) {
          flushWrapper();
          finalChildren.push(h("div", { class: "feature-code" }, node));
          continue;
        }
      }

      if (node.type === "mdxJsxFlowElement") {
        // code group is a special case and is feature by default
        if (node.name === "CodeGroup") {
          flushWrapper();
          finalChildren.push(h("div", { class: "feature-code" }, node));
          continue;
        }
      }

      // check if node contains img element or is the img element itself
      let imgNode: MdxJsxFlowElement | undefined;
      const imgNodesParent = node;
      if (node.children === undefined) continue;
      if (
        node.type === "mdxJsxFlowElement" &&
        node.name === "__CustomImage__"
      ) {
        imgNode = node as MdxJsxFlowElement;
      } else {
        for (const child of node.children) {
          if (
            child.type === "mdxJsxFlowElement" &&
            child.name === "__CustomImage__"
          ) {
            imgNode = child as MdxJsxFlowElement;
            break;
          }
        }
      }

      if (imgNode == null) {
        wrapQueue.push(node);
        continue;
      }

      // find the featuretype attribute
      const featureTypeAttr = imgNode.attributes.find(
        (attr) => attr.type === "mdxJsxAttribute" && attr.name === "featuretype"
      ) as MdxJsxAttribute | undefined;

      if (featureTypeAttr?.value == null) {
        wrapQueue.push(h("figure", [imgNodesParent]));
        continue;
      }

      if (typeof featureTypeAttr.value !== "string") {
        throw new Error("featuretype attribute must be a string");
      }

      flushWrapper();
      finalChildren.push(
        h(
          "figure",
          { class: ["feature", "feature-" + featureTypeAttr.value] },
          [imgNodesParent]
        )
      );
    }

    flushWrapper();
    tree.children = finalChildren;
  };
}
