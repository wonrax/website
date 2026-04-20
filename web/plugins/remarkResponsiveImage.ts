import type { Options as AcornOpts } from "acorn";
import type { Program } from "estree";
import { parse } from "acorn";
import type { Root, Paragraph, Image, PhrasingContent } from "mdast";
import type { Parent } from "unist";
import type {
  MdxJsxAttribute,
  MdxJsxAttributeValueExpression,
  MdxJsxFlowElement,
  MdxJsxTextElement,
  MdxjsEsm,
} from "mdast-util-mdx";

type CustomImageNode = MdxJsxFlowElement;

function isParagraphNode(node: Parent["children"][number]): node is Paragraph {
  return node.type === "paragraph";
}

function isMarkdownImageNode(node: Parent["children"][number]): node is Image {
  return node.type === "image";
}

function parseCustomTitle(
  value: string | MdxJsxAttributeValueExpression | null | undefined
) {
  if (typeof value !== "string" || !value.startsWith("#")) {
    return {
      title: value,
      attributes: [] as MdxJsxAttribute[],
    };
  }

  let title: string | MdxJsxAttributeValueExpression | null | undefined;
  const attributes: MdxJsxAttribute[] = [];

  for (const rawAttribute of value.slice(1).split(";")) {
    if (rawAttribute.length === 0) continue;

    const separatorIndex = rawAttribute.indexOf("=");
    const key =
      separatorIndex === -1
        ? rawAttribute
        : rawAttribute.slice(0, separatorIndex).trim();
    const parsedValue =
      separatorIndex === -1 ? "" : rawAttribute.slice(separatorIndex + 1);

    if (key.length === 0) continue;
    if (key === "title") {
      title = parsedValue;
      continue;
    }

    attributes.push({
      type: "mdxJsxAttribute",
      name: key,
      value: parsedValue,
    });
  }

  return { title, attributes };
}

function isCustomImageElement(
  node: Parent["children"][number] | PhrasingContent
): node is MdxJsxFlowElement | MdxJsxTextElement {
  if (node.type !== "mdxJsxFlowElement" && node.type !== "mdxJsxTextElement") {
    return false;
  }

  const mdxNode = node as MdxJsxFlowElement | MdxJsxTextElement;
  return mdxNode.name === "astro-image" || mdxNode.name === "img";
}

function convertMarkdownImage(
  node: Image,
  captionChildren: PhrasingContent[] = []
): CustomImageNode {
  const { title, attributes } = parseCustomTitle(node.title);

  return {
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
        value: node.alt ?? "",
      },
      ...(title == null
        ? []
        : [
            {
              name: "title",
              type: "mdxJsxAttribute" as const,
              value: title,
            },
          ]),
      ...attributes,
    ],
    children: captionChildren as CustomImageNode["children"],
  };
}

function convertMdxImage(
  node: MdxJsxFlowElement | MdxJsxTextElement,
  captionChildren: PhrasingContent[] = []
): CustomImageNode {
  const attributes: MdxJsxAttribute[] = [];

  for (const attribute of node.attributes) {
    if (attribute.type !== "mdxJsxAttribute") {
      continue;
    }

    if (attribute.name !== "title") {
      attributes.push(attribute);
      continue;
    }

    const parsedTitle = parseCustomTitle(attribute.value);
    if (parsedTitle.title != null) {
      attributes.push({
        ...attribute,
        value: parsedTitle.title,
      });
    }
    attributes.push(...parsedTitle.attributes);
  }

  return {
    name: "__CustomImage__",
    type: "mdxJsxFlowElement",
    attributes,
    children: captionChildren as CustomImageNode["children"],
  };
}

function convertParagraph(paragraph: Paragraph): CustomImageNode | null {
  const [firstChild, ...captionChildren] = paragraph.children;
  if (firstChild == null) {
    return null;
  }

  if (firstChild.type === "image") {
    return convertMarkdownImage(firstChild, captionChildren);
  }

  if (isCustomImageElement(firstChild)) {
    return convertMdxImage(firstChild, captionChildren);
  }

  return null;
}

function transformChildren(children: Parent["children"]): Parent["children"] {
  const transformedChildren: Parent["children"] = [];

  for (const child of children) {
    if (isParagraphNode(child)) {
      const convertedParagraph = convertParagraph(child);
      if (convertedParagraph != null) {
        transformedChildren.push(convertedParagraph);
        continue;
      }
    }

    if (isMarkdownImageNode(child)) {
      transformedChildren.push(convertMarkdownImage(child));
      continue;
    }

    if (isCustomImageElement(child)) {
      transformedChildren.push(convertMdxImage(child));
      continue;
    }

    if ("children" in child && Array.isArray(child.children)) {
      child.children = transformChildren(child.children);
    }

    transformedChildren.push(child);
  }

  return transformedChildren;
}

// By default images in mdx are not responsive. This plugin preserves the
// existing authoring syntax while delegating image optimization to Astro.
export default function remarkResponsiveImage() {
  return (tree: Root) => {
    tree.children = transformChildren(tree.children) as Root["children"];
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
        ...(parse(jsString, acornOpts) as Program),
        type: "Program",
        sourceType: "module",
      },
    },
  };
}
