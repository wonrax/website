import type { Options as AcornOpts } from "acorn";
import type { Program } from "estree";
import { parse } from "acorn";
import type { Image, Root } from "mdast";
import type {
  MdxJsxFlowElement,
  MdxjsEsm,
  MdxJsxAttribute,
  MdxJsxAttributeValueExpression,
} from "mdast-util-mdx";
import { visit } from "unist-util-visit";

// By default images in mdx are not responsive. This plugin
// converts all images to our own responsive image components, which are
// responsive by default. It also allows for custom attributes to be passed
// to the image component via the title attribute.
export default function remarkResponsiveImage() {
  return (tree: Root) => {
    // TODO the logic to handle local images is not updated with remote images
    // e.g. appending the rest of parent's children into this node
    // please fix by code sharing and refactoring, otherwise using local images
    // won't work as expected
    visit(tree, "mdxJsxFlowElement", (node: MdxJsxFlowElement) => {
      if (node.name !== "astro-image" && node.name !== "img") {
        return;
      }

      // find the title attribute
      const titleAttr = node.attributes.find(
        (attr) => attr.type === "mdxJsxAttribute" && attr.name === "title"
      ) as MdxJsxAttribute | undefined;

      if (titleAttr?.value != null && typeof titleAttr.value === "string") {
        // indicates that the title holds extra attributes seperated by semicolon
        if (titleAttr.value.startsWith("#")) {
          let title: string | MdxJsxAttributeValueExpression | null | undefined;
          for (const attr of titleAttr.value.slice(1).split(";")) {
            const [key, value] = attr.split("=");
            if (key === "title") title = value;
            node.attributes.push({
              type: "mdxJsxAttribute",
              name: key,
              value,
            });
          }
          titleAttr.value = title;
        }
      }

      node.name = "__CustomImage__";
    });

    // Handle remote images that are ignored by astro
    visit(tree, "image", (node: Image, index, parent) => {
      // Ripped off from https://github.com/withastro/astro/blob/0b22bb9af45d888d2a6de563ffdc3b8ad1bc0731/packages/integrations/mdx/src/remark-images-to-component.ts
      // Build a component that's equivalent to <Image src={importName} alt={node.alt} title={node.title} />
      const componentElement: MdxJsxFlowElement = {
        name: "__CustomImage__",
        type: "mdxJsxFlowElement",
        attributes: [
          {
            name: "src",
            type: "mdxJsxAttribute",
            value: node.url,
          },
          {
            name: "alt",
            type: "mdxJsxAttribute",
            value: node.alt ?? "", // TODO if the alt is null, find the nearest text node and use it as alt
          },
        ],
        children: (parent?.children
          ? parent.children.slice(index == null ? 0 : index + 1)
          : []) as MdxJsxFlowElement["children"],
      };

      if (node.title != null) {
        // indicates that the title holds extra attributes seperated by semicolon
        if (node.title.startsWith("#")) {
          for (const attr of node.title.slice(1).split(";")) {
            const [key, value] = attr.split("=");
            componentElement.attributes.push({
              type: "mdxJsxAttribute",
              name: key,
              value,
            });
          }
        }
      }

      // Replace the image node with the new component
      // and ignore the rest of parent's children since they're already appended
      // to the new component
      if (index != null && parent != null) {
        parent.children = [componentElement];
      } else {
        console.warn("index is null");
      }
    });

    tree.children.unshift(
      jsToTreeNode(
        `import __CustomImage__ from "@/components/BlogResponsiveImage.astro";`
      ),
      jsToTreeNode(`import CodeGroup from "@/components/CodeGroup.tsx";`)
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
        ...(parse(jsString, acornOpts) as Program), // Cast the parsed result as Program
        type: "Program",
        sourceType: "module",
      },
    },
  };
}
