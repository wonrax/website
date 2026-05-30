import generate from "@/components/OgImage/generate";
import { getPostSlug } from "@/shared/blog";
import type { APIRoute, GetStaticPathsResult } from "astro";
import { getCollection } from "astro:content";

export async function getStaticPaths(): Promise<GetStaticPathsResult> {
  const thoughts = await getCollection("thoughts");
  return thoughts.map((t) => ({
    params: { slug: getPostSlug(t) },
    props: { frontmatter: t.data },
  }));
}

export const GET: APIRoute = async ({ props }) => {
  const frontmatter = props.frontmatter as {
    title?: string;
    published: string;
  };

  const dateLabel = new Date(frontmatter.published).toLocaleDateString(
    "en-UK",
    {
      year: "numeric",
      month: "short",
      day: "numeric",
    }
  );

  return await generate({ title: frontmatter.title ?? dateLabel });
};
