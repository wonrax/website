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
import {
  CODE_GROUP_COMPONENT_NAME,
  CODE_GROUP_IMPORT_SOURCE,
  CUSTOM_IMAGE_COMPONENT_NAME,
  CUSTOM_IMAGE_IMPORT_SOURCE,
  type CustomImageNode,
  isMdxJsxElement,
} from "./shared";

function isParagraphNode(node: Parent["children"][number]): node is Paragraph {
  return node.type === "paragraph";
}

function isMarkdownImageNode(node: Parent["children"][number]): node is Image {
  return node.type === "image";
}

export function parseImageMeta(
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
  if (!isMdxJsxElement(node)) {
    return false;
  }

  return node.name === "astro-image" || node.name === "img";
}

function convertMarkdownImage(
  node: Image,
  captionChildren: PhrasingContent[] = []
): CustomImageNode {
  const { title, attributes } = parseImageMeta(node.title);

  return {
    name: CUSTOM_IMAGE_COMPONENT_NAME,
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
  const attributes: typeof node.attributes = [];

  for (const attribute of node.attributes) {
    if (attribute.type !== "mdxJsxAttribute") {
      attributes.push(attribute);
      continue;
    }

    if (attribute.name !== "title") {
      attributes.push(attribute);
      continue;
    }

    const parsedTitle = parseImageMeta(attribute.value);
    if (parsedTitle.title != null) {
      attributes.push({
        ...attribute,
        value: parsedTitle.title,
      });
    }
    attributes.push(...parsedTitle.attributes);
  }

  return {
    name: CUSTOM_IMAGE_COMPONENT_NAME,
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

function hasNodeNamed(children: Parent["children"], name: string): boolean {
  for (const child of children) {
    if (isMdxJsxElement(child) && child.name === name) {
      return true;
    }

    if ("children" in child && Array.isArray(child.children)) {
      if (hasNodeNamed(child.children as Parent["children"], name)) {
        return true;
      }
    }
  }

  return false;
}

export function hasMdxImport(tree: Root, source: string): boolean {
  return tree.children.some(
    (child) =>
      child.type === "mdxjsEsm" &&
      typeof child.value === "string" &&
      child.value.includes(`from "${source}"`)
  );
}

function ensureMdxImport(tree: Root, localName: string, source: string): void {
  if (hasMdxImport(tree, source)) {
    return;
  }

  tree.children.unshift(jsToTreeNode(`import ${localName} from "${source}";`));
}

export default function remarkResponsiveImage() {
  return (tree: Root) => {
    tree.children = transformChildren(tree.children) as Root["children"];

    if (
      hasNodeNamed(
        tree.children as Parent["children"],
        CUSTOM_IMAGE_COMPONENT_NAME
      )
    ) {
      ensureMdxImport(
        tree,
        CUSTOM_IMAGE_COMPONENT_NAME,
        CUSTOM_IMAGE_IMPORT_SOURCE
      );
    }

    if (
      hasNodeNamed(
        tree.children as Parent["children"],
        CODE_GROUP_COMPONENT_NAME
      )
    ) {
      ensureMdxImport(
        tree,
        CODE_GROUP_COMPONENT_NAME,
        CODE_GROUP_IMPORT_SOURCE
      );
    }
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
    value: jsString,
    data: {
      estree: {
        ...(parse(jsString, acornOpts) as Program),
        type: "Program",
        sourceType: "module",
      },
    },
  };
}
