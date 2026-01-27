import {
  createSignal,
  onMount,
  onCleanup,
  For,
  Show,
  type JSXElement,
} from "solid-js";
import { toast } from "solid-sonner";
import config from "@/config";

interface FeedItem {
  id: number;
  title: string;
  url: string;
  score: number;
  created_at: string | null;
  sources: string[];
}

interface FeedSnapshot {
  items: FeedItem[];
}

interface FeedEvent {
  type: "NewEntries";
  data: { count: number };
}

export default function RecommenderFeed(): JSXElement {
  const [items, setItems] = createSignal<FeedItem[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [err, setErr] = createSignal<string | null>(null);
  const [newItemsCount, setNewItemsCount] = createSignal(0);
  const [offset, setOffset] = createSignal(0);
  const [hasMore, setHasMore] = createSignal(true);
  const [loadingMore, setLoadingMore] = createSignal(false);

  const LIMIT = 20;

  const fetchFeed = async (currentOffset: number, append = false) => {
    try {
      const resp = await fetch(
        `${config.API_URL}/feed?offset=${currentOffset}&limit=${LIMIT}`
      );
      if (!resp.ok) {
        throw new Error(`API error: ${resp.status}`);
      }
      const data: FeedSnapshot = await resp.json();

      if (append) {
        setItems((prev) => [...prev, ...data.items]);
      } else {
        setItems(data.items);
      }

      setHasMore(data.items.length === LIMIT);
      setErr(null);
    } catch (e) {
      const message = e instanceof Error ? e.message : "Failed to load feed";
      setErr(message);
      toast.error(message, { duration: 5000 });
    }
  };

  const loadMore = async () => {
    if (loadingMore() || !hasMore()) return;
    setLoadingMore(true);
    const newOffset = offset() + LIMIT;
    await fetchFeed(newOffset, true);
    setOffset(newOffset);
    setLoadingMore(false);
  };

  const refresh = async () => {
    setLoading(true);
    setOffset(0);
    setNewItemsCount(0);
    await fetchFeed(0);
    setLoading(false);
  };

  onMount(() => {
    toast.loading("Fetching recommendations...", {
      id: "recommender-fetch",
      duration: Infinity,
    });

    fetchFeed(0).then(() => {
      setLoading(false);
      toast.dismiss("recommender-fetch");
    });

    const eventSource = new EventSource(`${config.API_URL}/feed/stream`);

    eventSource.onmessage = (event) => {
      try {
        const feedEvent: FeedEvent = JSON.parse(event.data);
        if (feedEvent.type === "NewEntries") {
          setNewItemsCount((prev) => prev + feedEvent.data.count);
          toast.info(`${feedEvent.data.count} new items available`, {
            duration: 5000,
          });
        }
      } catch {
        console.error("Failed to parse SSE event:", event.data);
      }
    };

    eventSource.onerror = () => {
      console.warn("SSE connection error, will retry automatically");
    };

    onCleanup(() => {
      eventSource.close();
    });
  });

  const getWebsiteUrl = (url: string): string => {
    try {
      const parsed = new URL(url);
      return parsed.hostname.replace(/^www\./, "");
    } catch {
      return url;
    }
  };

  const formatDate = (dateStr: string | null): string => {
    if (!dateStr) return "";
    return new Date(dateStr).toLocaleDateString(undefined, {
      year: "numeric",
      month: "long",
      day: "numeric",
    });
  };

  return (
    <>
      <Show when={newItemsCount() > 0}>
        <div class="new-items-banner">
          <span>{newItemsCount()} new items available</span>
          <button onClick={refresh} class="refresh-btn">
            Refresh
          </button>
        </div>
      </Show>

      <Show when={!loading()} fallback={<p>Loading…</p>}>
        <Show when={!err() || items().length > 0} fallback={<p>{err()}</p>}>
          <p style={{ color: "var(--text-body-light)" }}>
            Personalized article recommendations based on reading history and
            trending content from Hacker News and Lobsters.
          </p>
          <ul class="feed-list">
            <For each={items()}>
              {(item) => (
                <li class="feed-entry">
                  <span class="feed-date">{formatDate(item.created_at)}</span>
                  <a
                    href={item.url}
                    rel="noopener noreferrer"
                    class="feed-title"
                  >
                    {item.title}
                  </a>
                  <div class="feed-meta">
                    <a
                      href={`https://${getWebsiteUrl(item.url)}`}
                      rel="noopener noreferrer"
                      class="feed-source"
                    >
                      {getWebsiteUrl(item.url)}
                    </a>
                    <Show when={item.sources.length > 0}>
                      <span class="feed-sources">
                        via{" "}
                        <For each={item.sources}>
                          {(source, i) => (
                            <>
                              <span class="source-tag">{source}</span>
                              {i() < item.sources.length - 1 && ", "}
                            </>
                          )}
                        </For>
                      </span>
                    </Show>
                  </div>
                </li>
              )}
            </For>
          </ul>
          <Show when={hasMore()}>
            <button
              onClick={loadMore}
              disabled={loadingMore()}
              class="load-more-btn"
            >
              {loadingMore() ? "Loading..." : "Load more"}
            </button>
          </Show>
        </Show>
      </Show>
    </>
  );
}
