import generate from "@/components/OgImage/generate";
import type { APIRoute, GetStaticPathsResult } from "astro";
import type { Frontmatter } from "@/layouts/BlogPostLayout.astro";

export async function getStaticPaths(): Promise<GetStaticPathsResult> {
  const allPosts = import.meta.glob<Frontmatter>("../**/*.mdx");
  return await Promise.all(
    Object.keys(allPosts).map(async (slug) => {
      return {
        params: {
          slug: slug.replace("../", "").replace(".mdx", "").replace(".md", ""),
        },
      };
    }),
  );
}

export const GET: APIRoute = async ({ params, request }) => {
  const { frontmatter }: { frontmatter: Frontmatter } = await import(
    `../${params.slug}.mdx`
  );

  return await generate({
    title: frontmatter.title,
    description: frontmatter.description,
  });
};
