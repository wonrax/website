import { defineConfig } from "astro/config";
import tailwind from "@astrojs/tailwind";

import { visit } from "unist-util-visit";
import type { MdxJsxFlowElement } from "mdast-util-mdx";

import mdx from "@astrojs/mdx";
import remarkFeatureElement from "remark-feature-element";

// https://astro.build/config
export default defineConfig({
  markdown: {
    syntaxHighlight: false,
    remarkPlugins: [
      () => {
        return (tree) => {
          visit(tree, "mdxJsxFlowElement", (node: MdxJsxFlowElement) => {
            if (node.name != "__AstroImage__") {
              return;
            }

            console.log(node);

            node.name = "paragraph";
          });
        };
      },
    ],
    rehypePlugins: [remarkFeatureElement],
  },
  integrations: [tailwind(), mdx()],
});
