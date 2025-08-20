// Shared RSS feed parser for both server (Astro) and client (React)
export interface RSSItem {
  title?: string;
  link?: string;
  pubDate?: string;
}

export function parseFeed(xml: string): RSSItem[] {
  const doc = new window.DOMParser().parseFromString(xml, "text/xml");
  const items = Array.from(doc.querySelectorAll("item"));
  return items.map((item) => ({
    title: item.querySelector("title")?.textContent ?? undefined,
    link: item.querySelector("link")?.textContent ?? undefined,
    pubDate: item.querySelector("pubDate")?.textContent ?? undefined,
  }));
}
