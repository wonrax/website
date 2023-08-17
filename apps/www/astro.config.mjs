import { defineConfig } from "astro/config";
import image from "@astrojs/image";
import tailwind from "@astrojs/tailwind";
import mdx from "@astrojs/mdx";
import sitemap from "@astrojs/sitemap";
import react from "@astrojs/react";
import rehypePrettyCode from "rehype-pretty-code";
import remarkFeatureElement from "remark-feature-element";

const options = {
  // Use one of Shiki's packaged themes
  theme: {
    light: "min-light",
    dark: "github-dark",
  },

  // Keep the background or use a custom background color?
  keepBackground: false,

  // Callback hooks to add custom logic to nodes when visiting
  // them.
  onVisitLine(node) {
    // Prevent lines from collapsing in `display: grid` mode, and
    // allow empty lines to be copy/pasted
    if (node.children.length === 0) {
      node.children = [{ type: "text", value: " " }];
    }
  },
  onVisitHighlightedLine(node) {
    // Each line node by default has `class="line"`.
    node.properties.className.push("highlighted");
  },
  onVisitHighlightedWord(node) {
    // Each word node has no className by default.
    node.properties.className = ["word"];
  },
};

// https://astro.build/config
export default defineConfig({
  site: "https://hhai.dev",
  markdown: {
    syntaxHighlight: false,
    // remarkPlugins: [
    //   [
    //     remarkCodeExtra,
    //     {
    //       // Add a link to stackoverflow if there is one in the meta
    //       transform: (node) =>
    //         node.meta
    //           ? {
    //               after: [
    //                 {
    //                   type: "element",
    //                   tagName: "a",
    //                   properties: {
    //                     href: node.meta,
    //                   },
    //                   children: [
    //                     {
    //                       type: "text",
    //                       value: "View on Stack Overflow",
    //                     },
    //                   ],
    //                 },
    //               ],
    //             }
    //           : null,
    //     },
    //   ],
    // ],
    remarkPlugins: [],
    rehypePlugins: [remarkFeatureElement, [rehypePrettyCode, options]],
    // syntaxHighlight: "shiki",
    // shikiConfig: {
    //   theme: "dracula-soft",
    // },
  },
  integrations: [
    image({ serviceEntryPoint: "@astrojs/image/sharp" }),
    tailwind(),
    react(),
    mdx(),
    sitemap(),
  ],
});
