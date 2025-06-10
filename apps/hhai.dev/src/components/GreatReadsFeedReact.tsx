/** @jsxImportSource react */

import config from "@/config";
import { useEffect, useState, useRef } from "react";
import styles from "../pages/great-reads/GreatReadsFeed.module.scss";
import { parseFeed, type RSSItem } from "../shared/parseRssFeed";

interface Props {
  initialItems?: RSSItem[];
}

interface AnimatedItem extends RSSItem {
  status?: "normal" | "new";
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

export default function GreatReadsFeed({ initialItems = [] }: Props) {
  const [items, setItems] = useState<AnimatedItem[]>(
    initialItems.map((item) => ({ ...item, status: "normal" }))
  );
  const [loading, setLoading] = useState(initialItems.length === 0);
  const [err, setErr] = useState<string | null>(null);
  const prevItemsRef = useRef<RSSItem[]>(initialItems);

  useEffect(() => {
    fetch(`${config.API_URL}/great-reads-feed`)
      .then((resp) => resp.text())
      .then((xml) => {
        const nextItems = parseFeed(xml);
        const prev = prevItemsRef.current;
        const prevLinks = new Set(prev.map((i) => i.link));
        const merged: AnimatedItem[] = [];
        nextItems.forEach((item) => {
          merged.push({
            ...item,
            status: prevLinks.has(item.link) ? "normal" : "new",
          });
        });
        setItems(merged);
        prevItemsRef.current = nextItems;
        setTimeout(() => {
          setItems((current) =>
            current.map((item) =>
              item.status === "new" ? { ...item, status: "normal" } : item
            )
          );
        }, 1500);
        setLoading(false);
      })
      .catch((e) => {
        setErr("Failed to load feed");
        console.error("Failed to load great reads feed: ", e);
        setLoading(false);
      });
  }, []);

  if (loading) return <p>Loading…</p>;
  if (err && !items) return <p>{err}</p>;

  return (
    <>
      <p style={{ color: "var(--text-body-light)" }}>
        A selection of interesting articles, papers, and resources curated by
        me.
      </p>
      <ul className={styles["reading-list"]}>
        {items.map((item) => (
          <li
            className={[
              styles["reading-entry"],
              item.status === "new" && styles["new-entry"],
            ]
              .filter(Boolean)
              .join(" ")}
            key={item.link}
          >
            <span className={styles["reading-date"]}>
              {item.pubDate
                ? new Date(item.pubDate).toLocaleDateString(undefined, {
                    year: "numeric",
                    month: "long",
                    day: "numeric",
                  })
                : ""}
            </span>
            <a
              href={item.link}
              target="_blank"
              rel="noopener noreferrer"
              className={styles["reading-title"]}
            >
              {item.title}
            </a>
            {item.link && (
              <span className={styles["reading-source"]}>
                <a
                  href={getWebsiteUrl(item.link)}
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  {getWebsiteUrl(item.link)}
                </a>
              </span>
            )}
          </li>
        ))}
      </ul>
    </>
  );
}
