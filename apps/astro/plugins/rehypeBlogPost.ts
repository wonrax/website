import type { Element } from "hast";
import { h } from "hastscript";

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
        finalChildren.push(h("MaxWidth", wrapQueue));
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
      let imgNode;
      let imageIsNested = true;
      if (node.children === undefined) continue;
      if (node.name === "__CustomImage__") {
        imgNode = node;
        imageIsNested = false;
      } else {
        for (const child of node.children) {
          if (child.name === "__CustomImage__") {
            imgNode = node;
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
        children: Array<any>,
        classes: string[] | string = []
      ) => {
        // Find the image caption
        let imageCaption: Array<any> | undefined = undefined;
        if (imageIsNested) {
          if (children.length > 1) imageCaption = children.slice(1);
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

        console.log("image caption", imageCaption);

        const imageCaptionElement =
          imageCaption && h("div", { class: "image-caption" }, imageCaption);

        let childrenToPush;
        if (imageIsNested) childrenToPush = [children[0], imageCaptionElement];
        else childrenToPush = [node, imageCaptionElement];

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

      console.log("img node", node);
      console.log(
        "next to img node",
        tree.children[tree.children.indexOf(node) + 1]
      );
      console.log(
        "next 2 to img node",
        tree.children[tree.children.indexOf(node) + 2]
      );

      if (!imgNode.properties || !imgNode.properties.alt) {
        pushImgElement("div", wrapQueue, node.children);
        continue;
      }

      // First part of alt text is the actual alt text
      // and the second part is the feature type
      const altComponents = imgNode.properties.alt.toString().split("|");

      if (altComponents.length <= 1) {
        pushImgElement("div", wrapQueue, node.children);
        continue;
      }

      if (!node.properties) node.properties = {};
      imgNode.properties.alt = altComponents[0].trim();
      const featureType = altComponents[1].split("-")[1].trim();
      imgNode.properties["feature-type"] = featureType; // We also need to set this in order to enable responsive images

      flushWrapper();
      pushImgElement(
        "div",
        finalChildren,
        node.children,
        altComponents.slice(1)
      );
    }

    flushWrapper();
    tree.children = finalChildren;
  };
}
