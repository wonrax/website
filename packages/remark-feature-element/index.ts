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
        if (!imgNode.properties || !imgNode.properties.alt) {
          wrapQueue.push(node);
          continue;
        }

        const components = imgNode.properties.alt.toString().split("|");
        if (components.length <= 1) {
          wrapQueue.push(node);
          continue;
        }

        if (!node.properties) node.properties = {};
        imgNode.properties.alt = components[0].trim();
        const featureType = components[1].split("-")[1].trim();
        imgNode.properties["feature-type"] = featureType; // We also need to set this in order to enable responsive images

        flushWrapper();
        finalChildren.push(
          h("div", { class: components.slice(1) }, node.children)
        );
      } else {
        wrapQueue.push(node);
      }
    }

    flushWrapper();
    tree.children = finalChildren;
  };
}
