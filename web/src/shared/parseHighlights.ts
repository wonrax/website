// Shared highlight parser and types for both server (Astro) and client (React)

export interface HighlightItem {
  id: string;
  title: string;
  text: string;
  note?: string;
  color: string;
  created_at: string;
  link: string;
  tags: string[];
}

export interface RSSItem {
  title?: string;
  link?: string;
  pubDate?: string;
}

// Helper function to format highlight date
export function formatHighlightDate(dateStr: string): string {
  try {
    return new Date(dateStr).toLocaleDateString(undefined, {
      year: "numeric",
      month: "long",
      day: "numeric",
    });
  } catch {
    return "";
  }
}

// Helper function to get website URL from link
export function getWebsiteUrl(link: string | undefined): string {
  if (!link) return "";
  try {
    const url = new URL(link);
    return url.origin;
  } catch {
    return link;
  }
}

// Convert highlights to RSS-like items for backwards compatibility during migration
export function highlightsToRSSItems(highlights: HighlightItem[]): RSSItem[] {
  return highlights.map((highlight) => ({
    title: highlight.title,
    link: highlight.link,
    pubDate: highlight.created_at,
  }));
}
