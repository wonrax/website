import type { Element } from "hast";
import { h } from "hastscript";
import type { Root } from "mdast";
import type { Data, Node } from "unist";
import { visit } from "unist-util-visit";

type DirectiveNode = Node & {
  type: "containerDirective" | "leafDirective" | "textDirective";
  name?: string | null;
  attributes?: Record<string, string | number | boolean | null | undefined>;
  data?: Data;
};

function isDirectiveNode(node: Node): node is DirectiveNode {
  return (
    node.type === "containerDirective" ||
    node.type === "leafDirective" ||
    node.type === "textDirective"
  );
}

export function remarkDirectiveHtml() {
  return (tree: Root) => {
    visit(tree, (node) => {
      if (!isDirectiveNode(node) || node.name == null) {
        return;
      }

      const data = node.data ?? (node.data = {});
      const hast = h(node.name, node.attributes ?? {}) as Element;

      data.hName = hast.tagName;
      data.hProperties = hast.properties;
    });
  };
}
