import type { Element } from "hast";
import { h } from "hastscript";
import type { MdxJsxFlowElement, MdxJsxAttribute } from "mdast-util-mdx";

// add extra custom modification like feature image etc.
export default function rehypeBlogPost() {
  return (tree: any) => {
    // The queue that holds the normal elements that will be wrapped in a
    // MaxWidth component, whose width is reading line length. When a feature
    // image is encountered, the queue is flushed and the image is added
    // without the MaxWidth wrapper. This is inspired by react.dev
    let wrapQueue: Element[] = [];

    // The final children that will be set back to the tree
    const finalChildren: Element[] = [];

    function flushWrapper(): void {
      if (wrapQueue.length > 0) {
        finalChildren.push(
          h("div", { class: "reading-line-width" }, wrapQueue),
        );
        wrapQueue = [];
      }
    }

    for (let index = 0; index < tree.children.length; index++) {
      const node = tree.children[index];

      // ignore import statements in the beginning of the file
      if (node.type === "mdxjsEsm") {
        finalChildren.push(node);
        continue;
      }

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

      // code block is a special case and is feature by default
      if (
        node.tagName === "figure" &&
        node.properties?.["data-rehype-pretty-code-figure"] === ""
      ) {
        flushWrapper();
        finalChildren.push(h("div", { class: "feature-code" }, node));
        continue;
      }

      // code group is a special case and is feature by default
      if (node.type === "mdxJsxFlowElement" && node.name === "CodeGroup") {
        flushWrapper();
        finalChildren.push(h("div", { class: "feature-code" }, node));
        continue;
      }

      // check if node contains img element or is the img element itself
      let imgNode: MdxJsxFlowElement | undefined;
      if (node.children === undefined) continue;
      if (node.name === "__CustomImage__") {
        imgNode = node;
      } else {
        for (const child of node.children) {
          if (child.name === "__CustomImage__") {
            imgNode = child;
            break;
          }
        }
      }

      if (imgNode == null) {
        wrapQueue.push(imgNode);
        continue;
      }

      const pushImgElement = (
        as: string,
        queue: any[],
        imgNode: MdxJsxFlowElement,
        classes: string[] | string | undefined = undefined,
      ): void => {
        // Find the image caption

        queue.push(h(as, { class: classes }, [imgNode as any]));
      };

      // find the featuretype attribute
      const featureTypeAttr = imgNode.attributes.find(
        (attr) => attr.type === "mdxJsxAttribute" && attr.name == "featuretype",
      ) as MdxJsxAttribute | undefined;

      if (featureTypeAttr?.value == null) {
        pushImgElement("figure", wrapQueue, imgNode);
        continue;
      }

      if (typeof featureTypeAttr.value !== "string") {
        throw new Error("featuretype attribute must be a string");
      }

      flushWrapper();
      pushImgElement("figure", finalChildren, imgNode, [
        "feature",
        "feature-" + featureTypeAttr.value,
      ]);
    }

    flushWrapper();
    tree.children = finalChildren;
  };
}
