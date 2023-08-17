import { defineDocumentType, makeSource } from "@contentlayer/source-files";
import rehypePrettyCode from "rehype-pretty-code";
import remarkFeatureElement from "remark-feature-element";
import remarkGfm from "remark-gfm";
import rehypeSlug from "rehype-slug";
import rehypeAutolinkHeadings from "rehype-autolink-headings";
import { fromMarkdown } from "mdast-util-from-markdown";
import { Heading } from "mdast-util-from-markdown/lib";

export const BlogPost = defineDocumentType(() => ({
  name: "BlogPost",
  filePathPattern: `blog/**/*.mdx`,
  contentType: "mdx",
  fields: {
    title: { type: "string", required: true },
    description: { type: "string", required: false },
    published: { type: "date", required: true },
    updated: { type: "date", required: false },
    tags: { type: "list", of: { type: "string" }, required: false },
  },
  computedFields: {
    slug: {
      type: "string",
      resolve: (post) =>
        `${
          post._raw.flattenedPath.split("/")[
            post._raw.flattenedPath.split("/").length - 1
          ]
        }`,
    },
    toc: {
      type: "string",
      resolve: (post) => {
        type NestedHeading = {
          slug: string;
          title: string;
          depth: number;
          children: NestedHeading[];
        };
        const tree = fromMarkdown(post.body.raw);
        const headings = tree.children.filter(
          (node) => node.type === "heading"
        ) as Heading[];
        const customHeadings = visit(headings);
        function visit(flat_children: Heading[]): NestedHeading[] {
          const children: NestedHeading[] = [];
          for (let i = 0; i < flat_children.length; i++) {
            const queue: Heading[] = [];
            const current = flat_children[i];
            i = i + 1;
            while (i < flat_children.length) {
              const next = flat_children[i];
              if (next.depth <= current.depth) {
                i = i - 1;
                break;
              }
              queue.push(next);
              i = i + 1;
            }
            const _children = visit(queue);
            if (!current.children || current.children[0].type !== "text") {
              console.warn("Cannot get text of this heading", current);
              continue;
            }
            children.push({
              title: current.children[0].value,
              children: _children,
              depth: current.depth,
              slug: current.children[0].value
                .split(" ")
                .join("-")
                .toLocaleLowerCase(), // TODO use github-slugger to handle collision
            });
          }
          return children;
        }

        // Limitation: nested field types in computed fields are not supported yet
        // See: https://github.com/contentlayerdev/contentlayer/issues/149
        return JSON.stringify(customHeadings);
      },
    },
  },
}));

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
  onVisitLine(node) {
    // Prevent lines from collapsing in `display: grid` mode, and
    // allow empty lines to be copy/pasted
    if (node.children.length === 0) {
      node.children = [{ type: "text", value: " " }];
    }
    if (!node.properties.className) {
      node.properties.className = ["line"]
    }
  },
  onVisitHighlightedLine(node) {
    node.properties.className.push("highlighted");
  },
  onVisitHighlightedWord() {
  },
};

export default makeSource({
  contentDirPath: "content",
  documentTypes: [BlogPost],
  mdx: {
    remarkPlugins: [remarkGfm],
    rehypePlugins: [
      rehypeSlug,
      [rehypeAutolinkHeadings, { behavior: "append" }],
      remarkFeatureElement,
      [rehypePrettyCode, codeHighlightOptions],
    ],
  },
});
