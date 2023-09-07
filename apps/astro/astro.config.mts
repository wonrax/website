import { defineConfig, sharpImageService } from "astro/config";
import tailwind from "@astrojs/tailwind";

import { visit } from "unist-util-visit";
import type { MdxJsxFlowElement } from "mdast-util-mdx";

import mdx from "@astrojs/mdx";
import remarkFeatureElement from "remark-feature-element";
import {
  jsToTreeNode,
  remarkImageToComponent,
} from "./remark-images-to-components";

// https://astro.build/config
export default defineConfig({
  markdown: {
    syntaxHighlight: false,
    remarkPlugins: [
      remarkImageToComponent,
      () => {
        return (tree) => {
          visit(tree, "mdxJsxFlowElement", (node: MdxJsxFlowElement) => {
            if (node.name != "__AstroImage__") {
              return;
            }
            console.log(node);
            // node.name = "__CustomImage__";
          });

          tree.children.unshift(
            jsToTreeNode(`import __CustomImage__ from "../../../Image.astro";`)
          );
        };
      },
    ],
    rehypePlugins: [remarkFeatureElement],
  },
  integrations: [tailwind(), mdx()],
  image: {
    service: sharpImageService(),
    domains: ["astro.build"],
  },
});
