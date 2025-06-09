/** @jsxImportSource react */

import config from "@/config";
import { useEffect, useState } from "react";
import styles from "../pages/great-reads/GreatReadsFeed.module.scss";

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
    <>
      <p style={{ color: "var(--text-body-light)" }}>
        A selection of interesting articles, papers, and resources curated by
        me.
      </p>
      <ul className={styles["reading-list"]}>
        {items.map((item) => (
          <li className={styles["reading-entry"]} key={item.link}>
            <div className={styles["reading-row"]}>
              <a
                href={item.link}
                target="_blank"
                rel="noopener noreferrer"
                className={styles["reading-title"]}
              >
                {item.title}
              </a>
              <hr className={styles["reading-divider"]} />
              <span className={styles["reading-date"]}>
                {item.pubDate
                  ? new Date(item.pubDate).toLocaleDateString(undefined, {
                      year: "numeric",
                      month: "long",
                      day: "numeric",
                    })
                  : ""}
              </span>
            </div>
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
