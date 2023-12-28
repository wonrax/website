import mdx from "@astrojs/mdx";
import react from "@astrojs/react";
import { defineConfig, sharpImageService } from "astro/config";
import path, { dirname } from "path";
import rehypePrettyCode from "rehype-pretty-code";
import { fileURLToPath } from "url";
import rehypeBlogPost from "./plugins/rehypeBlogPost";
import remarkResponsiveImage from "./plugins/remarkResponsiveImage";
import remarkCalloutDirectives from "@microflash/remark-callout-directives";
import remarkDirective from "remark-directive";
import { remarkDirectiveHtml } from "./plugins/remarkDirective";
import rehypeKatex from "rehype-katex";
import remarkMath from "remark-math";

const __dirname = dirname(fileURLToPath(import.meta.url));

const codeHighlightOptions = {
  // Use one of Shiki's packaged themes
  theme: {
    light: "min-light",
    dark: "min-dark",
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
  integrations: [mdx(), react()],
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
