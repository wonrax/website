import { glob } from "astro/loaders";
import { defineCollection, z } from "astro:content";

const blog = defineCollection({
  loader: glob({ pattern: "**/*.{md,mdx}", base: "./src/content/blog" }),
  schema: z.object({
    title: z.string(),
    description: z.string(),
    // FIXME: consider using `published: z.coerce.date(),`
    published: z.string(),
    updated: z.string().optional(),
    tags: z.array(z.string()),
    slug: z.string().optional(),
    isDraft: z.boolean().optional(),
    hidden: z.boolean().optional(),
    ogImageUrl: z.string().optional(),
    layout: z.enum(["BlogPostLayout"]).optional(),
    seriesId: z.string().optional(),
    seriesTitle: z.string().optional(),
    seriesDescription: z.string().optional(),
  }),
});

export const collections = { blog };
