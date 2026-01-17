import generate from "@/components/OgImage/generate";
import { getPostSlug } from "@/shared/blog";
import type { APIRoute, GetStaticPathsResult } from "astro";
import { getCollection } from "astro:content";

export async function getStaticPaths(): Promise<GetStaticPathsResult> {
  const allPosts = await getCollection("blog");
  return allPosts.map((post) => ({
    params: {
      slug: getPostSlug(post),
    },
    props: {
      frontmatter: post.data,
    },
  }));
}

export const GET: APIRoute = async ({ props }) => {
  const frontmatter = props.frontmatter as {
    title: string;
    description: string;
  };

  return await generate({
    title: frontmatter.title,
    description: frontmatter.description,
  });
};
