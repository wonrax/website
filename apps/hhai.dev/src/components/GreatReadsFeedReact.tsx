/** @jsxImportSource react */

import config from "@/config";
import { useEffect, useState } from "react";

interface RSSItem {
  title?: string;
  link?: string;
  pubDate?: string;
}

function parseFeed(xml: string): RSSItem[] {
  const doc = new window.DOMParser().parseFromString(xml, "text/xml");
  const items = Array.from(doc.querySelectorAll("item"));
  return items.map((item) => ({
    title: item.querySelector("title")?.textContent ?? undefined,
    link: item.querySelector("link")?.textContent ?? undefined,
    pubDate: item.querySelector("pubDate")?.textContent ?? undefined,
  }));
}

function getWebsiteUrl(link: string | undefined) {
  if (!link) return "";
  try {
    const url = new URL(link);
    return url.origin;
  } catch {
    return link;
  }
}

export default function GreatReadsFeed() {
  const [items, setItems] = useState<RSSItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    fetch(`${config.API_URL}/great-reads-feed`)
      .then((resp) => resp.text())
      .then((xml) => {
        setItems(parseFeed(xml));
        setLoading(false);
      })
      .catch(() => {
        setErr("Failed to load feed");
        setLoading(false);
      });
  }, []);

  if (loading) return <p>Loading…</p>;
  if (err) return <p>{err}</p>;

  return (
    <div>
      <h2>Great Reads</h2>
      <ul>
        {items.map((item) => (
          <li key={item.link}>
            <a href={item.link} target="_blank" rel="noopener noreferrer">
              {item.title}
            </a>
            <br />
            <small>
              {item.pubDate && new Date(item.pubDate).toLocaleDateString()}{" "}
              &nbsp; | &nbsp;
              {item.link && (
                <a
                  href={getWebsiteUrl(item.link)}
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  {getWebsiteUrl(item.link)}
                </a>
              )}
            </small>
          </li>
        ))}
      </ul>
    </div>
  );
}
