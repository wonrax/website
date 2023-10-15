import { h } from "hastscript";
import { visit } from "unist-util-visit";

// Turn directives to tag name and class names
// Grabbed from https://github.com/remarkjs/remark-directive#use
export function remarkDirectiveHtml() {
  return function (tree: any) {
    visit(tree, function (node) {
      if (
        node.type === "containerDirective" ||
        node.type === "leafDirective" ||
        node.type === "textDirective"
      ) {
        const data = node.data || (node.data = {});
        const hast: any = h(node.name, node.attributes || {});

        data.hName = hast.tagName;
        data.hProperties = hast.properties;
      }
    });
  };
}
