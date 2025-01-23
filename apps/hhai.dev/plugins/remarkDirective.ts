import { h } from "hastscript";
import { visit } from "unist-util-visit";

// Turn directives to tag name and class names
// Grabbed from https://github.com/remarkjs/remark-directive#use
export function remarkDirectiveHtml() {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  return function (tree: any) {
    visit(tree, function (node) {
      if (
        node.type === "containerDirective" ||
        node.type === "leafDirective" ||
        node.type === "textDirective"
      ) {
        const data = node.data || (node.data = {});
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const hast = h(node.name, node.attributes || {}) as any;

        data.hName = hast.tagName;
        data.hProperties = hast.properties;
      }
    });
  };
}
