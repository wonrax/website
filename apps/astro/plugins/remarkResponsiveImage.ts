import type { Options as AcornOpts } from "acorn";
import { parse } from "acorn";
import type { MdxJsxFlowElement, MdxjsEsm } from "mdast-util-mdx";
import { visit } from "unist-util-visit";

export default function remarkResponsiveImage() {
  return (tree: any) => {
    visit(tree, "mdxJsxFlowElement", (node: MdxJsxFlowElement) => {
      if (node.name != "astro-image" && node.name != "img") {
        return;
      }
      node.name = "__CustomImage__";
    });

    tree.children.unshift(
      jsToTreeNode(
        `import __CustomImage__ from "@/components/ResponsiveImage.astro";`
      )
    );
  };
}

export function jsToTreeNode(
  jsString: string,
  acornOpts: AcornOpts = {
    ecmaVersion: "latest",
    sourceType: "module",
  }
): MdxjsEsm {
  return {
    type: "mdxjsEsm",
    value: "",
    data: {
      estree: {
        body: [],
        ...parse(jsString, acornOpts),
        type: "Program",
        sourceType: "module",
      },
    },
  };
}
