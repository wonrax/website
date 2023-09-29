import { defineConfig, sharpImageService } from "astro/config";
import tailwind from "@astrojs/tailwind";
import path from "path";

import { visit } from "unist-util-visit";
import type { MdxJsxFlowElement } from "mdast-util-mdx";

import mdx from "@astrojs/mdx";
import remarkFeatureElement from "remark-feature-element";
import {
  jsToTreeNode,
  // remarkImageToComponent,
} from "./remark-images-to-components";

import { dirname } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));

// https://astro.build/config
export default defineConfig({
  markdown: {
    syntaxHighlight: false,
    remarkPlugins: [
      // remarkImageToComponent,
      () => {
        return (tree) => {
          visit(tree, "mdxJsxFlowElement", (node: MdxJsxFlowElement) => {
            console.log(node);
            if (node.name != "astro-image" && node.name != "img") {
              return;
            }
            node.name = "__CustomImage__";
          });

          tree.children.unshift(
            jsToTreeNode(
              `import __CustomImage__ from "@/components/ResponsiveImage.astro";`
            )
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
  vite: {
    resolve: {
      alias: {
        "@": path.resolve(__dirname, "./src"),
      },
    },
  },
});
