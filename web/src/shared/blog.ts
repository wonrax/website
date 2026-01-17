import { getCollection, getEntry, type CollectionEntry } from "astro:content";

type BlogEntry = CollectionEntry<"blog">;

type BlogSeriesEntry = {
  title: string;
  slug: string;
};

type BlogSeriesInfo = {
  title: string;
  description: string | null;
  posts: BlogSeriesEntry[];
  position: number;
  total: number;
};

/** Get the slug for a blog post entry based on its frontmatter or file path. */
export function getPostSlug(entry: {
  data: { slug?: string };
  filePath?: string;
}): string {
  if (entry.data.slug) {
    return entry.data.slug.split("/").pop() ?? entry.data.slug;
  } else if (entry.filePath) {
    const parts = entry.filePath.split("/");
    const fileName = parts[parts.length - 1];
    return fileName.replace(/\.[^/.]+$/, ""); // Remove file extension
  }

  throw new Error("Entry must have either a data.slug or filePath defined.");
}

export async function getBlogPostSlugById(
  id: BlogEntry["id"]
): Promise<string> {
  const entry = await getEntry("blog", id);
  if (!entry) {
    throw new Error(`Missing blog entry for id: ${id}`);
  }

  return getPostSlug(entry);
}

export async function getBlogPostRoutes(): Promise<BlogEntry[]> {
  const posts = await getCollection("blog");
  return posts;
}

export async function getSeriesInfo(
  post: BlogEntry
): Promise<BlogSeriesInfo | null> {
  if (!post.data.seriesId) {
    return null;
  }

  const seriesPosts = (await getCollection("blog")).filter(
    (entry) => entry.data.seriesId === post.data.seriesId
  );

  const sortedSeriesPosts = [...seriesPosts].sort((a, b) => {
    return (
      new Date(a.data.published).getTime() -
      new Date(b.data.published).getTime()
    );
  });

  const firstEntry = sortedSeriesPosts[0];
  if (!firstEntry || !firstEntry.data.seriesTitle) {
    throw new Error(
      `Series ${post.data.seriesId} requires the first post to define seriesTitle.`
    );
  }

  const extraSeriesDefinitions = sortedSeriesPosts.filter(
    (entry, index) => index !== 0 && Boolean(entry.data.seriesTitle)
  );

  if (extraSeriesDefinitions.length > 0) {
    throw new Error(
      `Series ${post.data.seriesId} has multiple seriesTitle definitions.`
    );
  }

  if (post.data.seriesTitle && firstEntry.id !== post.id) {
    throw new Error(
      `Series ${post.data.seriesId} metadata must live on the first post.`
    );
  }

  const seriesIndex = sortedSeriesPosts.findIndex(
    (entry) => entry.id === post.id
  );

  return {
    title: firstEntry.data.seriesTitle,
    description: firstEntry.data.seriesDescription ?? null,
    posts: sortedSeriesPosts.map((entry) => ({
      title: entry.data.title,
      slug: getPostSlug(entry),
    })),
    position: seriesIndex + 1,
    total: sortedSeriesPosts.length,
  };
}

export function ensureSeriesConsistency(post: BlogEntry): void {
  if (!post.data.seriesId && post.data.seriesTitle) {
    throw new Error("seriesTitle requires seriesId.");
  }
}
