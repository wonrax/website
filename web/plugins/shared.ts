import type {
  MdxJsxAttribute,
  MdxJsxFlowElement,
  MdxJsxTextElement,
} from "mdast-util-mdx";

export const CUSTOM_IMAGE_COMPONENT_NAME = "__CustomImage__";
export const CUSTOM_IMAGE_IMPORT_SOURCE =
  "@/components/BlogResponsiveImage.astro";
export const CODE_GROUP_COMPONENT_NAME = "CodeGroup";
export const CODE_GROUP_IMPORT_SOURCE = "@/components/CodeGroup.tsx";
export const FEATURE_TYPE_ATTRIBUTE = "featuretype";

export type CustomImageNode = MdxJsxFlowElement & {
  name: typeof CUSTOM_IMAGE_COMPONENT_NAME;
};

export function isMdxJsxElement(
  node: { type?: string } | null | undefined
): node is MdxJsxFlowElement | MdxJsxTextElement {
  return (
    node?.type === "mdxJsxFlowElement" || node?.type === "mdxJsxTextElement"
  );
}

export function isCustomImageNode(
  node: { type?: string; name?: string | null } | null | undefined
): node is CustomImageNode {
  return isMdxJsxElement(node) && node.name === CUSTOM_IMAGE_COMPONENT_NAME;
}

export function getStringAttribute(
  node: {
    attributes?: (MdxJsxAttribute | { type: string })[];
  },
  name: string
): string | undefined {
  const attribute = node.attributes?.find(
    (candidate): candidate is MdxJsxAttribute =>
      candidate.type === "mdxJsxAttribute" &&
      "name" in candidate &&
      candidate.name === name
  );

  return typeof attribute?.value === "string" ? attribute.value : undefined;
}
