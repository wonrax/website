import mdx from "@astrojs/mdx";
import tailwind from "@astrojs/tailwind";
import { defineConfig, sharpImageService } from "astro/config";
import path, { dirname } from "path";
import rehypePrettyCode from "rehype-pretty-code";
import { fileURLToPath } from "url";
import rehypeBlogPost from "./plugins/rehypeBlogPost";
import remarkResponsiveImage from "./plugins/remarkResponsiveImage";

const __dirname = dirname(fileURLToPath(import.meta.url));

const codeHighlightOptions = {
  // Use one of Shiki's packaged themes
  theme: {
    light: "github-light",
    dark: "github-dark",
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
      node.properties.className = ["line"];
    }
  },
  onVisitHighlightedLine(node: any) {
    node.properties.className.push("highlighted");
  },
  onVisitHighlightedWord() {},
};

// https://astro.build/config
export default defineConfig({
  markdown: {
    syntaxHighlight: false,
    remarkPlugins: [remarkResponsiveImage],
    rehypePlugins: [rehypeBlogPost, [rehypePrettyCode, codeHighlightOptions]],
  },
  integrations: [tailwind(), mdx()],
  image: {
    service: sharpImageService(),
    domains: ["astro.build", "picsum.photos"],
  },
  vite: {
    resolve: {
      alias: {
        "@": path.resolve(__dirname, "./src"),
      },
    },
  },
});
