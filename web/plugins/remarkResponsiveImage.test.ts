import { describe, expect, test } from "vitest";
import type { MdxjsEsm } from "mdast-util-mdx";
import type { Root } from "mdast";
import remarkResponsiveImage, {
  hasMdxImport,
  jsToTreeNode,
  parseImageMeta,
} from "./remarkResponsiveImage";
import {
  CODE_GROUP_COMPONENT_NAME,
  CODE_GROUP_IMPORT_SOURCE,
  CUSTOM_IMAGE_COMPONENT_NAME,
  CUSTOM_IMAGE_IMPORT_SOURCE,
} from "./shared";

function getImportSources(tree: Root): string[] {
  return tree.children
    .filter((node): node is MdxjsEsm => node.type === "mdxjsEsm")
    .map((node) => node.value);
}

describe("parseImageMeta", () => {
  test("keeps plain titles untouched", () => {
    expect(parseImageMeta("cover image")).toEqual({
      title: "cover image",
      attributes: [],
    });
  });

  test("parses markdown image metadata attributes", () => {
    expect(parseImageMeta("#width=1920;height=816;featuretype=md")).toEqual({
      title: undefined,
      attributes: [
        {
          type: "mdxJsxAttribute",
          name: "width",
          value: "1920",
        },
        {
          type: "mdxJsxAttribute",
          name: "height",
          value: "816",
        },
        {
          type: "mdxJsxAttribute",
          name: "featuretype",
          value: "md",
        },
      ],
    });
  });

  test("supports title plus metadata attributes", () => {
    expect(parseImageMeta("#title=hello;width=640")).toEqual({
      title: "hello",
      attributes: [
        {
          type: "mdxJsxAttribute",
          name: "width",
          value: "640",
        },
      ],
    });
  });
});

describe("jsToTreeNode", () => {
  test("keeps the source string on the mdx import node", () => {
    const node = jsToTreeNode(`import Foo from "./foo";`);
    expect(node.value).toBe(`import Foo from "./foo";`);
  });
});

describe("hasMdxImport", () => {
  test("detects matching import sources", () => {
    const tree: Root = {
      type: "root",
      children: [
        jsToTreeNode(`import Foo from "${CUSTOM_IMAGE_IMPORT_SOURCE}";`),
      ],
    };

    expect(hasMdxImport(tree, CUSTOM_IMAGE_IMPORT_SOURCE)).toBe(true);
    expect(hasMdxImport(tree, CODE_GROUP_IMPORT_SOURCE)).toBe(false);
  });
});

describe("remarkResponsiveImage", () => {
  test("converts markdown images to the custom image component and injects its import", () => {
    const tree: Root = {
      type: "root",
      children: [
        {
          type: "paragraph",
          children: [
            {
              type: "image",
              url: "https://example.com/cat.jpg",
              alt: "cat",
              title: "#width=640;height=480",
            },
            {
              type: "text",
              value: "caption text",
            },
          ],
        },
      ],
    };

    remarkResponsiveImage()(tree);

    expect(getImportSources(tree)).toContain(
      `import ${CUSTOM_IMAGE_COMPONENT_NAME} from "${CUSTOM_IMAGE_IMPORT_SOURCE}";`
    );

    const imageNode = tree.children.find(
      (node) =>
        node.type === "mdxJsxFlowElement" &&
        node.name === CUSTOM_IMAGE_COMPONENT_NAME
    );

    expect(imageNode).toMatchObject({
      type: "mdxJsxFlowElement",
      name: CUSTOM_IMAGE_COMPONENT_NAME,
      attributes: [
        {
          type: "mdxJsxAttribute",
          name: "src",
          value: "https://example.com/cat.jpg",
        },
        { type: "mdxJsxAttribute", name: "alt", value: "cat" },
        { type: "mdxJsxAttribute", name: "width", value: "640" },
        { type: "mdxJsxAttribute", name: "height", value: "480" },
      ],
      children: [{ type: "text", value: "caption text" }],
    });
  });

  test("injects the code group import only when code groups are present", () => {
    const tree: Root = {
      type: "root",
      children: [
        {
          type: "mdxJsxFlowElement",
          name: CODE_GROUP_COMPONENT_NAME,
          attributes: [],
          children: [],
        },
      ],
    };

    remarkResponsiveImage()(tree);

    expect(getImportSources(tree)).toContain(
      `import ${CODE_GROUP_COMPONENT_NAME} from "${CODE_GROUP_IMPORT_SOURCE}";`
    );
  });

  test("does not duplicate imports that already exist", () => {
    const imageImport = `import ${CUSTOM_IMAGE_COMPONENT_NAME} from "${CUSTOM_IMAGE_IMPORT_SOURCE}";`;
    const codeGroupImport = `import ${CODE_GROUP_COMPONENT_NAME} from "${CODE_GROUP_IMPORT_SOURCE}";`;

    const tree: Root = {
      type: "root",
      children: [
        jsToTreeNode(imageImport),
        jsToTreeNode(codeGroupImport),
        {
          type: "paragraph",
          children: [
            {
              type: "image",
              url: "/cat.jpg",
              alt: "cat",
              title: null,
            },
          ],
        },
        {
          type: "mdxJsxFlowElement",
          name: CODE_GROUP_COMPONENT_NAME,
          attributes: [],
          children: [],
        },
      ],
    };

    remarkResponsiveImage()(tree);

    expect(getImportSources(tree)).toEqual([imageImport, codeGroupImport]);
  });
});
