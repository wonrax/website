import satori from "satori";
import sharp from "sharp";
import OgImage from "@/components/OgImage";
import type { APIRoute, AstroGlobal } from "astro";
import type { Frontmatter } from "@/layouts/BlogPostLayout.astro";

declare const Astro: AstroGlobal;

export async function getStaticPaths() {
  const allPosts = import.meta.glob<Frontmatter>("./**/*.mdx");
  return allPosts.map((post) => ({
    params: { slug: await post.frontmatter.title },
    props: await post,
  }));
}

export const GET: APIRoute = async ({ params, request }) => {
  const slug = params.slug;
  // make a cache out of this for efficient build and bandwith
  const interNormal = await fetch(
    "https://cdn.jsdelivr.net/npm/@fontsource/inter@5.0.16/files/inter-latin-400-normal.woff"
  );
  const interBold = await fetch(
    "https://cdn.jsdelivr.net/npm/@fontsource/inter@5.0.16/files/inter-latin-700-normal.woff"
  );

  const svg = await satori(OgImage(), {
    width: 1200,
    height: 630,
    fonts: [
      {
        name: "Inter",
        data: await interNormal.arrayBuffer(),
        weight: 400,
        style: "normal",
      },
      {
        name: "Inter",
        data: await interBold.arrayBuffer(),
        weight: 700,
        style: "normal",
      },
    ],
  });

  const png = await sharp(Buffer.from(svg)).png().toBuffer();

  return new Response(png, {
    headers: {
      "Content-Type": "image/png",
    },
  });
};
