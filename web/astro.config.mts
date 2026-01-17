import mdx from "@astrojs/mdx";
import react from "@astrojs/react";
import remarkCalloutDirectives from "@microflash/remark-callout-directives";
import { defineConfig, sharpImageService } from "astro/config";
import rehypeKatex from "rehype-katex";
import rehypePrettyCode from "rehype-pretty-code";
import remarkDirective from "remark-directive";
import remarkMath from "remark-math";
import type { Element } from "hast";
import rehypeBlogPost from "./plugins/rehypeBlogPost";
import { remarkDirectiveHtml } from "./plugins/remarkDirective";
import remarkResponsiveImage from "./plugins/remarkResponsiveImage";
import solid from "@astrojs/solid-js";

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
  onVisitLine(node: Element) {
    // Prevent lines from collapsing in `display: grid` mode, and
    // allow empty lines to be copy/pasted
    if (node.children.length === 0) {
      node.children = [{ type: "text", value: " " }];
    }
    if (node.properties?.className == null) {
      node.properties = { ...node.properties, className: ["code-block-line"] };
    }

    // remove data-line attribute
    delete node.properties?.["data-line"];
  },
  onVisitHighlightedLine(node: Element) {
    if (node.properties?.className == null) {
      node.properties = { ...node.properties, className: [] };
    }
    if (Array.isArray(node.properties.className)) {
      node.properties.className.push("highlighted");
    } else if (
      typeof node.properties.className === "string" ||
      typeof node.properties.className === "number"
    ) {
      node.properties.className = [node.properties.className, "highlighted"];
    } else {
      node.properties.className = ["highlighted"];
    }

    // remove data-highlighted-line attribute
    delete node.properties["data-highlighted-line"];
  },
  onVisitHighlightedWord() {},
  onVisitTitle(node: Element) {
    node.properties = { ...node.properties, className: ["code-block-title"] };
    delete node.properties["data-rehype-pretty-code-title"];
  },
};

// https://astro.build/config
export default defineConfig({
  site: "https://wrx.sh",
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
    solid({ exclude: "**/*React.tsx" }),
    react({
      include: ["**/*React.tsx"],
    }),
  ],
  image: {
    service: sharpImageService(),
    domains: ["files.wrx.sh", "res.cloudinary.com"],
  },
  vite: {
    optimizeDeps: { exclude: ["@resvg/resvg-js"] },
    server: {
      proxy: {
        "/api": {
          target: "http://localhost:3000",
          rewrite: (path) => path.replace(/^\/api/, ""),
        },
      },
    },
  },
});
