import { describe, expect, test } from "vitest";
import type { Root } from "hast";
import rehypeCodeGroup from "./rehypeCodeGroup";
import {
  CODE_GROUP_COMPONENT_NAME,
  CODE_GROUP_TITLE_ATTRIBUTE,
} from "./shared";

function getCodeGroupFigure(tree: Root) {
  const codeGroup = tree.children[0];
  if (
    codeGroup == null ||
    codeGroup.type !== "mdxJsxFlowElement" ||
    codeGroup.children[0] == null
  ) {
    throw new Error("Expected a CodeGroup node with at least one child");
  }

  return codeGroup.children[0];
}

describe("rehypeCodeGroup", () => {
  test("extracts code titles into figure attributes and removes title nodes", () => {
    const tree: Root = {
      type: "root",
      children: [
        {
          type: "mdxJsxFlowElement",
          name: CODE_GROUP_COMPONENT_NAME,
          attributes: [],
          children: [
            {
              type: "element",
              tagName: "figure",
              properties: {
                "data-rehype-pretty-code-figure": "",
              },
              children: [
                {
                  type: "element",
                  tagName: "div",
                  properties: {
                    className: ["code-block-title"],
                  },
                  children: [{ type: "text", value: "foo/bar.ts" }],
                },
                {
                  type: "element",
                  tagName: "pre",
                  properties: {},
                  children: [],
                },
              ],
            },
          ],
        },
      ],
    };

    rehypeCodeGroup()(tree);

    const figure = getCodeGroupFigure(tree);
    expect(figure).toMatchObject({
      type: "element",
      tagName: "figure",
      properties: {
        "data-rehype-pretty-code-figure": "",
        [CODE_GROUP_TITLE_ATTRIBUTE]: "bar.ts",
      },
      children: [
        {
          type: "element",
          tagName: "pre",
        },
      ],
    });
  });

  test("fills empty titles when no pretty-code title exists", () => {
    const tree: Root = {
      type: "root",
      children: [
        {
          type: "mdxJsxFlowElement",
          name: CODE_GROUP_COMPONENT_NAME,
          attributes: [],
          children: [
            {
              type: "element",
              tagName: "figure",
              properties: {
                "data-rehype-pretty-code-figure": "",
              },
              children: [],
            },
          ],
        },
      ],
    };

    rehypeCodeGroup()(tree);

    expect(getCodeGroupFigure(tree)).toMatchObject({
      properties: {
        [CODE_GROUP_TITLE_ATTRIBUTE]: "",
      },
    });
  });
});
