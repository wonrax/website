/** @jsxImportSource react */

import config from "@/config";
import { useEffect, useState } from "react";
import styles from "../pages/great-reads/GreatReadsFeed.module.scss";
import { parseFeed, type RSSItem } from "../shared/parseRssFeed";

interface Props {
  initialItems?: RSSItem[];
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
  const [items, setItems] = useState<RSSItem[]>(initialItems);
  const [loading, setLoading] = useState(initialItems.length === 0);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    fetch(`${config.API_URL}/great-reads-feed`)
      .then((resp) => resp.text())
      .then((xml) => {
        setItems(parseFeed(xml));
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
          <li className={styles["reading-entry"]} key={item.link}>
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
