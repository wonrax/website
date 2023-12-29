import mdx from "@astrojs/mdx";
import react from "@astrojs/react";
import remarkCalloutDirectives from "@microflash/remark-callout-directives";
import { defineConfig, sharpImageService } from "astro/config";
import path, { dirname } from "path";
import rehypeKatex from "rehype-katex";
import rehypePrettyCode from "rehype-pretty-code";
import remarkDirective from "remark-directive";
import remarkMath from "remark-math";
import { fileURLToPath } from "url";
import rehypeBlogPost from "./plugins/rehypeBlogPost";
import { remarkDirectiveHtml } from "./plugins/remarkDirective";
import remarkResponsiveImage from "./plugins/remarkResponsiveImage";
import solid from "@astrojs/solid-js";

const __dirname = dirname(fileURLToPath(import.meta.url));

const codeHighlightOptions = {
  // Use one of Shiki's packaged themes
  theme: {
    light: "rose-pine-dawn",
    dark: "rose-pine-moon",
  },

  // Keep the background or use a custom background color?
  keepBackground: false,

  // Callback hooks to add custom logic to nodes when visiting
  // them.
  onVisitLine(node: any) {
    // Prevent lines from collapsing in `display: grid` mode, and
    // allow empty lines to be copy/pasted
    if (node.children.length === 0) {
      node.children = [{ type: "text", value: " " }];
    }
    if (!node.properties.className) {
      node.properties.className = ["code-block-line"];
    }

    // remove data-line attribute
    delete node.properties["data-line"];
  },
  onVisitHighlightedLine(node: any) {
    node.properties.className.push("highlighted");

    // remove data-highlighted-line attribute
    delete node.properties["data-highlighted-line"];
  },
  onVisitHighlightedWord() {},
  onVisitTitle(node: any) {
    node.properties.className = ["code-block-title"];
    delete node.properties["data-rehype-pretty-code-title"];
  },
};

// https://astro.build/config
export default defineConfig({
  markdown: {
    syntaxHighlight: false,
    remarkPlugins: [
      remarkMath,
      remarkDirective,
      [
        remarkCalloutDirectives,
        {
          callouts: {
            note: {
              title: "Note",
              hint: "",
            },
            warning: {
              title: "Warning",
              hint: "",
            },
          },
        },
      ],
      remarkDirectiveHtml,
      remarkResponsiveImage,
    ],
    rehypePlugins: [
      rehypeKatex,
      [rehypePrettyCode, codeHighlightOptions],
      rehypeBlogPost,
    ],
  },
  integrations: [
    mdx(),
    react({ include: "**/*React.tsx" }),
    solid({ include: "**/*Solid.tsx" }),
  ],
  image: {
    service: sharpImageService(),
    domains: ["share.hhai.dev", "res.cloudinary.com"],
  },
  vite: {
    resolve: {
      alias: {
        "@": path.resolve(__dirname, "./src"),
      },
    },
    optimizeDeps: { exclude: ["@resvg/resvg-js"] },
  },
});
