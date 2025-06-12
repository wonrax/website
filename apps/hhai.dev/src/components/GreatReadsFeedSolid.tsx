import { createSignal, onMount, For, Show, type JSXElement } from "solid-js";
import { toast } from "solid-sonner";
import config from "@/config";
import styles from "../pages/great-reads/GreatReadsFeed.module.scss";
import { parseFeed, type RSSItem } from "../shared/parseRssFeed";
import type { HighlightItem } from "../shared/parseHighlights";
import { getWebsiteUrl } from "../shared/parseHighlights";

interface Props {
  initialItems?: RSSItem[];
  initialHighlights?: HighlightItem[];
}

interface ArticleHighlights {
  link: string;
  title: string;
  highlights: HighlightItem[];
}

interface MergedArticle {
  link: string;
  title: string;
  date: string;
  highlights?: HighlightItem[];
  status?: "normal" | "new";
}

export default function GreatReadsFeed(props: Props): JSXElement {
  const initialItems = props.initialItems || [];
  const initialHighlights = props.initialHighlights || [];

  // Group highlights by article
  const groupHighlightsByArticle = (
    highlights: HighlightItem[]
  ): ArticleHighlights[] => {
    const grouped = new Map<string, ArticleHighlights>();

    highlights.forEach((highlight) => {
      const key = highlight.link;
      if (!grouped.has(key)) {
        grouped.set(key, {
          link: highlight.link,
          title: highlight.title,
          highlights: [],
        });
      }
      const article = grouped.get(key)!;
      article.highlights.push(highlight);
    });

    return Array.from(grouped.values());
  };

  // Merge articles with highlights and RSS items
  const mergeArticles = (
    groupedHighlights: ArticleHighlights[],
    rssItems: RSSItem[]
  ): MergedArticle[] => {
    const articlesMap = new Map<string, MergedArticle>();

    // First, add all RSS items (this defines our collection)
    rssItems.forEach((item) => {
      if (item.link) {
        articlesMap.set(item.link, {
          link: item.link,
          title: item.title || "",
          date: item.pubDate || "",
          status: "normal",
        });
      }
    });

    // Then, add highlights only for articles that exist in RSS
    groupedHighlights.forEach((article) => {
      const existingArticle = articlesMap.get(article.link);
      if (existingArticle) {
        // Update with highlights and use the latest highlight date
        articlesMap.set(article.link, {
          ...existingArticle,
          highlights: article.highlights,
        });
      }
      // Skip highlights for articles not in RSS collection
    });

    // Sort by date (newest first)
    return Array.from(articlesMap.values()).sort(
      (a, b) => new Date(b.date).getTime() - new Date(a.date).getTime()
    );
  };

  const [articles, setArticles] = createSignal<MergedArticle[]>(
    mergeArticles(groupHighlightsByArticle(initialHighlights), initialItems)
  );
  const [loading, setLoading] = createSignal(false); // Start as false since we have initial content
  const [err, setErr] = createSignal<string | null>(null);

  let prevArticles = mergeArticles(
    groupHighlightsByArticle(initialHighlights),
    initialItems
  );

  onMount(() => {
    // Only show loading if we have no initial content
    if (initialItems.length === 0 && initialHighlights.length === 0) {
      setLoading(true);
    }

    // Capture current articles length before async operation
    const currentArticlesLength = articles().length;

    // Show fetching status with toast using a consistent ID
    toast.loading("Fetching latest articles and highlights...", {
      id: "great-reads-fetch",
      duration: Infinity,
    });

    // Always try to fetch both highlights and RSS, then merge them
    Promise.all([
      fetch(`${config.API_URL}/great-reads-highlights`).then((resp) => {
        if (!resp.ok) throw new Error("Highlights API failed");
        return resp.json() as Promise<HighlightItem[]>;
      }), // Fallback to initial highlights on error

      fetch(`${config.API_URL}/great-reads-feed`)
        .then((resp) => resp.text())
        .then((xml) => parseFeed(xml)), // Fallback to initial RSS items on error
      // artificially delay to simulate loading
      new Promise((resolve) => setTimeout(resolve, 3000)),
    ])
      .then(([highlightsData, rssData]) => {
        const newArticles = mergeArticles(
          groupHighlightsByArticle(highlightsData),
          rssData
        );

        // Mark new articles
        const prevLinks = new Set(prevArticles.map((a) => a.link));
        const articlesWithStatus = newArticles.map((article) => ({
          ...article,
          status: prevLinks.has(article.link)
            ? ("normal" as const)
            : ("new" as const),
        }));

        setArticles(articlesWithStatus);
        prevArticles = newArticles;

        // Reset new status after animation
        setTimeout(() => {
          setArticles((current) =>
            current.map((article) =>
              article.status === "new"
                ? { ...article, status: "normal" }
                : article
            )
          );
        }, 1500);

        setLoading(false);
        setErr(null); // Clear any previous errors on successful load
        toast.dismiss("great-reads-fetch");
      })
      .catch((e) => {
        // Update the same toast to show error
        if (currentArticlesLength === 0) {
          setErr("Failed to load feed");
          toast.error("Failed to load articles and highlights", {
            id: "great-reads-fetch",
          });
        } else {
          toast.error("Failed to refresh content, showing cached data", {
            id: "great-reads-fetch",
          });
        }
        console.error("Failed to load great reads feed: ", e);
        setLoading(false);
      });
  });

  return (
    <Show when={!loading()} fallback={<p>Loading…</p>}>
      <Show when={!err() || articles().length > 0} fallback={<p>{err()}</p>}>
        <p style={{ color: "var(--text-body-light)" }}>
          A selection of interesting articles, papers, and resources curated by
          me. Articles with highlights show my notes and selected passages.
        </p>
        <ul class={styles["reading-list"]}>
          <For each={articles()}>
            {(article) => (
              <li
                class={[
                  styles["reading-entry"],
                  article.status === "new" && styles["new-entry"],
                ]
                  .filter(Boolean)
                  .join(" ")}
              >
                <span class={styles["reading-date"]}>
                  {article.date
                    ? new Date(article.date).toLocaleDateString(undefined, {
                        year: "numeric",
                        month: "long",
                        day: "numeric",
                      })
                    : ""}
                </span>
                <a
                  href={article.link}
                  target="_blank"
                  rel="noopener noreferrer"
                  class={styles["reading-title"]}
                >
                  {article.title}
                </a>

                <span class={styles["reading-source"]}>
                  <a
                    href={getWebsiteUrl(article.link)}
                    target="_blank"
                    rel="noopener noreferrer"
                  >
                    {getWebsiteUrl(article.link)}
                  </a>
                </span>

                <Show
                  when={article.highlights && article.highlights.length > 0}
                >
                  <div class={styles["highlights-container"]}>
                    <For each={article.highlights}>
                      {(highlight) => (
                        <div class={styles["highlight-content"]}>
                          <blockquote
                            class={styles["highlight-text"]}
                            style={{ "white-space": "pre-line" }} // preserve line breaks
                            // style={{ "border-left-color": highlight.color }}
                          >
                            {highlight.text}
                          </blockquote>
                          <Show when={highlight.note}>
                            <div class={styles["highlight-note"]}>
                              <strong>Note:</strong> {highlight.note}
                            </div>
                          </Show>
                        </div>
                      )}
                    </For>
                    <Show when={article.highlights![0]?.tags.length > 0}>
                      <div class={styles["highlight-tags"]}>
                        <For each={article.highlights![0].tags}>
                          {(tag) => (
                            <span class={styles["highlight-tag"]}>{tag}</span>
                          )}
                        </For>
                      </div>
                    </Show>
                  </div>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </Show>
  );
}
