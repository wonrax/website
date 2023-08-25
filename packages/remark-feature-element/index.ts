import type { Element } from "hast";
import { select } from "hast-util-select";
import { h } from "hastscript";

export default function remarkFeatureElement(options) {
  return (tree) => {
    const children: Element[] = tree.children;

    let wrapQueue: Element[] = [];
    let finalChildren: Element[] = [];

    function flushWrapper() {
      if (wrapQueue.length > 0) {
        finalChildren.push(h("MaxWidth", wrapQueue));
        wrapQueue = [];
      }
    }

    for (const node of children) {
      if (node.tagName == "table") {
        flushWrapper();
        finalChildren.push(h("div", { class: "feature-table-md" }, node));
        continue;
      }

      // check if node contains img element
      const imgNode = select("img", node);
      if (imgNode) {
        const pushImgElement = (
          as: string,
          queue: Array<any>,
          children,
          classes: string[] | string = []
        ) => {
          queue.push(
            h(as, { class: classes }, [
              children[0],
              h(
                "p",
                { class: "image-caption" }, // this assumes that the first child of the node is the image
                children.slice(1).map((child) => {
                  if (child.type === "text") return child.value.trim();
                  else return child;
                })
              ),
            ])
          );
        };

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
        console.log(node);
        pushImgElement(
          "div",
          finalChildren,
          node.children,
          altComponents.slice(1)
        );
        // finalChildren.push(
        //   h("div", { class: altComponents.slice(1) }, [
        //     node.children[0],
        //     h(
        //       "p",
        //       { class: "image-caption" }, // this assumes that the first child of the node is the image
        //       node.children.slice(1).map((child) => {
        //         if (child.type === "text") return child.value.trim();
        //         else return child;
        //       })
        //     ),
        //   ])
        // );
      } else {
        wrapQueue.push(node);
      }
    }

    flushWrapper();
    tree.children = finalChildren;
  };
}
