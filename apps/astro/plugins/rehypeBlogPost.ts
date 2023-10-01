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
    let finalChildren: Element[] = [];

    function flushWrapper() {
      if (wrapQueue.length > 0) {
        finalChildren.push(
          h("div", { class: "reading-line-width" }, wrapQueue)
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
      if (node.tagName == "table") {
        flushWrapper();
        finalChildren.push(h("div", { class: "feature-table-md" }, node));
        continue;
      }

      // check if node contains img element or is the img element itself
      let imgNode: MdxJsxFlowElement | undefined = undefined;
      let parent: any = undefined;
      if (node.children === undefined) continue;
      if (node.name === "__CustomImage__") {
        imgNode = node;
      } else {
        for (const child of node.children) {
          if (child.name === "__CustomImage__") {
            imgNode = child;
            parent = node;
            break;
          }
        }
      }

      if (!imgNode) {
        wrapQueue.push(node);
        continue;
      }

      const pushImgElement = (
        as: string,
        queue: Array<any>,
        imgNode: MdxJsxFlowElement,
        classes: string[] | string | undefined = undefined
      ) => {
        // Find the image caption
        let imageCaption: Array<any> | undefined = undefined;
        if (parent && parent.children) {
          if (parent.children.length > 1)
            imageCaption = parent.children.slice(1);
        } else {
          // the image caption is the second next node if the next node
          // is a \n text node and the second next node is a blockquote
          if (tree.children.length > index + 2) {
            const nextNode = tree.children[index + 1];
            const secondNextNode = tree.children[index + 2];
            if (
              nextNode.type === "text" &&
              nextNode.value.trim() === "" &&
              secondNextNode.tagName === "blockquote"
            ) {
              imageCaption = secondNextNode.children;

              // remove the 2 nodes from the tree
              tree.children.splice(index + 1, 2);
            }
          }
        }

        const imageCaptionElement =
          imageCaption && h("div", { class: "image-caption" }, imageCaption);

        const childrenToPush = [imgNode as any, imageCaptionElement];

        queue.push(
          h(as, { class: classes }, [
            ...childrenToPush,
            // children[0],
            // h(
            //   "p",
            //   { class: "image-caption" },
            //   children.slice(1).map((child) => {
            //     if (child.type === "text") return child.value.trim();
            //     else return child;
            //   })
            // imageCaption
            // ),
          ])
        );
      };

      console.log("img node", JSON.stringify(imgNode, null, 2));

      // find the featuretype attribute
      const featureTypeAttr = imgNode.attributes.find(
        (attr) => attr.type === "mdxJsxAttribute" && attr.name == "featuretype"
      ) as MdxJsxAttribute | undefined;

      if (!featureTypeAttr || !featureTypeAttr.value) {
        pushImgElement("div", wrapQueue, imgNode);
        continue;
      }

      flushWrapper();
      pushImgElement("div", finalChildren, imgNode, [
        "feature",
        "feature-" + featureTypeAttr.value,
      ]);

      console.log(
        "next to img node",
        tree.children[tree.children.indexOf(node) + 1]
      );
      console.log(
        "next 2 to img node",
        tree.children[tree.children.indexOf(node) + 2]
      );
    }

    flushWrapper();
    tree.children = finalChildren;
  };
}
