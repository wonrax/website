import { describe, expect, test } from "vitest";
import type { Root } from "hast";
import rehypeBlogPost from "./rehypeBlogPost";
import { CUSTOM_IMAGE_COMPONENT_NAME } from "./shared";

describe("rehypeBlogPost", () => {
  test("groups consecutive reading-width blocks together", () => {
    const tree: Root = {
      type: "root",
      children: [
        {
          type: "element",
          tagName: "p",
          properties: {},
          children: [{ type: "text", value: "one" }],
        },
        {
          type: "element",
          tagName: "p",
          properties: {},
          children: [{ type: "text", value: "two" }],
        },
      ],
    };

    rehypeBlogPost()(tree);

    expect(tree.children).toHaveLength(1);
    expect(tree.children[0]).toMatchObject({
      type: "element",
      tagName: "div",
      properties: { className: ["reading-line-width"] },
    });
  });

  test("flushes reading-width content before featured tables", () => {
    const tree: Root = {
      type: "root",
      children: [
        {
          type: "element",
          tagName: "p",
          properties: {},
          children: [{ type: "text", value: "intro" }],
        },
        {
          type: "element",
          tagName: "table",
          properties: {},
          children: [],
        },
      ],
    };

    rehypeBlogPost()(tree);

    expect(tree.children).toHaveLength(2);
    expect(tree.children[0]).toMatchObject({
      type: "element",
      tagName: "div",
      properties: { className: ["reading-line-width"] },
    });
    expect(tree.children[1]).toMatchObject({
      type: "element",
      tagName: "div",
      properties: { className: ["feature-table"] },
    });
  });

  test("wraps plain custom images as reading-width figures", () => {
    const tree: Root = {
      type: "root",
      children: [
        {
          type: "mdxJsxFlowElement",
          name: CUSTOM_IMAGE_COMPONENT_NAME,
          attributes: [],
          children: [],
        },
      ],
    };

    rehypeBlogPost()(tree);

    expect(tree.children).toHaveLength(1);
    expect(tree.children[0]).toMatchObject({
      type: "element",
      tagName: "div",
      properties: { className: ["reading-line-width"] },
      children: [
        {
          type: "element",
          tagName: "figure",
        },
      ],
    });
  });

  test("wraps featured custom images with feature classes", () => {
    const tree: Root = {
      type: "root",
      children: [
        {
          type: "mdxJsxFlowElement",
          name: CUSTOM_IMAGE_COMPONENT_NAME,
          attributes: [
            {
              type: "mdxJsxAttribute",
              name: "featuretype",
              value: "md",
            },
          ],
          children: [],
        },
      ],
    };

    rehypeBlogPost()(tree);

    expect(tree.children).toHaveLength(1);
    expect(tree.children[0]).toMatchObject({
      type: "element",
      tagName: "figure",
      properties: { className: ["feature", "feature-md"] },
    });
  });

  test("flushes before feature images and starts a new reading group after them", () => {
    const tree: Root = {
      type: "root",
      children: [
        {
          type: "element",
          tagName: "p",
          properties: {},
          children: [{ type: "text", value: "before" }],
        },
        {
          type: "mdxJsxFlowElement",
          name: CUSTOM_IMAGE_COMPONENT_NAME,
          attributes: [
            {
              type: "mdxJsxAttribute",
              name: "featuretype",
              value: "lg",
            },
          ],
          children: [],
        },
        {
          type: "element",
          tagName: "p",
          properties: {},
          children: [{ type: "text", value: "after" }],
        },
      ],
    };

    rehypeBlogPost()(tree);

    expect(tree.children).toHaveLength(3);
    expect(tree.children[0]).toMatchObject({
      type: "element",
      tagName: "div",
      properties: { className: ["reading-line-width"] },
    });
    expect(tree.children[1]).toMatchObject({
      type: "element",
      tagName: "figure",
      properties: { className: ["feature", "feature-lg"] },
    });
    expect(tree.children[2]).toMatchObject({
      type: "element",
      tagName: "div",
      properties: { className: ["reading-line-width"] },
    });
  });

  test("treats code groups as feature code blocks", () => {
    const tree: Root = {
      type: "root",
      children: [
        {
          type: "mdxJsxFlowElement",
          name: "CodeGroup",
          attributes: [],
          children: [],
        },
      ],
    };

    rehypeBlogPost()(tree);

    expect(tree.children).toHaveLength(1);
    expect(tree.children[0]).toMatchObject({
      type: "element",
      tagName: "div",
      properties: { className: ["feature-code"] },
    });
  });

  test("recognizes feature images nested inside wrapper elements", () => {
    const tree: Root = {
      type: "root",
      children: [
        {
          type: "element",
          tagName: "p",
          properties: {},
          children: [
            {
              type: "mdxJsxFlowElement",
              name: CUSTOM_IMAGE_COMPONENT_NAME,
              attributes: [
                {
                  type: "mdxJsxAttribute",
                  name: "featuretype",
                  value: "sm",
                },
              ],
              children: [],
            },
          ],
        },
      ],
    };

    rehypeBlogPost()(tree);

    expect(tree.children).toHaveLength(1);
    expect(tree.children[0]).toMatchObject({
      type: "element",
      tagName: "figure",
      properties: { className: ["feature", "feature-sm"] },
    });
  });
});
