import { describe, expect, test } from "vitest";
import type { Root } from "mdast";
import { remarkDirectiveHtml } from "./remarkDirective";

describe("remarkDirectiveHtml", () => {
  test("maps directive nodes to hast metadata", () => {
    const tree: Root = {
      type: "root",
      children: [
        {
          type: "containerDirective",
          name: "aside",
          attributes: {
            class: "callout",
            "data-tone": "muted",
          },
          children: [],
        },
      ],
    };

    remarkDirectiveHtml()(tree);

    const directiveNode = tree.children[0] as {
      data?: { hName?: string; hProperties?: Record<string, unknown> };
    };

    expect(directiveNode.data?.hName).toBe("aside");
    expect(directiveNode.data?.hProperties).toMatchObject({
      className: ["callout"],
      dataTone: "muted",
    });
  });

  test("ignores non-directive nodes", () => {
    const tree: Root = {
      type: "root",
      children: [
        {
          type: "paragraph",
          children: [],
        },
      ],
    };

    remarkDirectiveHtml()(tree);

    expect(tree.children[0]).not.toHaveProperty("data");
  });
});
