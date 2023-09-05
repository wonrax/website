import type { MarkdownVFile } from "@astrojs/markdown-remark";
import type { Image, Parent } from "mdast";
import type { MdxJsxFlowElement, MdxjsEsm } from "mdast-util-mdx";
import { visit } from "unist-util-visit";
import type { Options as AcornOpts } from "acorn";
import { parse } from "acorn";

export function remarkImageToComponent() {
  return function (tree: any, file: MarkdownVFile) {
    if (!file.data.imagePaths) return;

    const importedImages = new Map<string, string>();

    visit(
      tree,
      "image",
      (node: Image, index: number | null, parent: Parent | null) => {
        // Use the imagePaths set from the remark-collect-images so we don't have to duplicate the logic for

        // Build a component that's equivalent to <Image src={importName} alt={node.alt} title={node.title} />
        const componentElement: MdxJsxFlowElement = {
          name: "__AstroImage__",
          type: "mdxJsxFlowElement",
          attributes: [
            {
              name: "src",
              type: "mdxJsxAttribute",
              value: node.url,
            },
            { name: "alt", type: "mdxJsxAttribute", value: node.alt || "" },
          ],
          children: [],
        };

        if (node.title) {
          componentElement.attributes.push({
            type: "mdxJsxAttribute",
            name: "title",
            value: node.title,
          });
        }

        parent!.children.splice(index!, 1, componentElement);
      }
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
