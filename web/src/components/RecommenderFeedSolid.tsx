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
import { timeSince } from "@/utils/time";

interface SourceInfo {
  key: string;
  score: number | null;
  external_id: string | null;
}

interface FeedItem {
  id: number;
  title: string;
  url: string;
  score: number;
  similarity_score: number | null;
  submitted_at: string | null;
  sources: SourceInfo[];
}

interface FeedSnapshot {
  items: FeedItem[];
}

interface FeedEvent {
  type: "NewEntries";
  data: { count: number };
}

type SourceFilter = "all" | "hacker_news" | "lobsters";
type RankingPreset = "balanced" | "newer_first" | "top_first" | "similar_first";

const SOURCE_OPTIONS: { value: SourceFilter; label: string }[] = [
  { value: "all", label: "All" },
  { value: "lobsters", label: "Lobsters" },
  { value: "hacker_news", label: "Hacker News" },
];

const RANKING_OPTIONS: {
  value: RankingPreset;
  label: string;
  description: string;
}[] = [
  { value: "balanced", label: "Balanced", description: "Mix of all signals" },
  {
    value: "top_first",
    label: "Top first",
    description: "Prioritize external score",
  },
  {
    value: "newer_first",
    label: "Newer first",
    description: "Prioritize freshness",
  },
  {
    value: "similar_first",
    label: "Similar to you",
    description: "Prioritize vector similarity",
  },
];

export default function RecommenderFeed(): JSXElement {
  const [items, setItems] = createSignal<FeedItem[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [err, setErr] = createSignal<string | null>(null);
  const [newItemsCount, setNewItemsCount] = createSignal(0);
  const [offset, setOffset] = createSignal(0);
  const [hasMore, setHasMore] = createSignal(true);
  const [loadingMore, setLoadingMore] = createSignal(false);

  const [sourceFilter, setSourceFilter] = createSignal<SourceFilter>("all");
  const [ranking, setRanking] = createSignal<RankingPreset>("balanced");

  const LIMIT = 20;

  const fetchFeed = async (currentOffset: number, append = false) => {
    try {
      const params = new URLSearchParams({
        offset: currentOffset.toString(),
        limit: LIMIT.toString(),
        source: sourceFilter(),
        ranking: ranking(),
      });
      const resp = await fetch(`${config.API_URL}/feed?${params}`);
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

  const handleSourceChange = (newSource: SourceFilter) => {
    setSourceFilter(newSource);
    setOffset(0);
    setLoading(true);
    fetchFeed(0).then(() => setLoading(false));
  };

  const handleRankingChange = (newRanking: RankingPreset) => {
    setRanking(newRanking);
    setOffset(0);
    setLoading(true);
    fetchFeed(0).then(() => setLoading(false));
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

  const formatRelativeTime = (dateStr: string | null): string => {
    if (!dateStr) return "";
    return timeSince(new Date(dateStr + "Z"));
  };

  const formatSimilarity = (score: number | null): string => {
    if (score === null) return "N/A%";
    return `${Math.round(score * 100)}%`;
  };

  const formatSourceLabel = (source: SourceInfo): string => {
    let label: string;
    if (source.key === "hacker-news") {
      label = "Hacker News";
    } else if (source.key === "lobsters") {
      label = "Lobsters";
    } else {
      label = source.key;
    }
    if (source.score !== null) {
      return `${label} (${Math.round(source.score)})`;
    }
    return label;
  };

  const getSourceDiscussionUrl = (source: SourceInfo): string | null => {
    if (!source.external_id) return null;
    if (source.key === "hacker-news") {
      return `https://news.ycombinator.com/item?id=${source.external_id}`;
    }
    if (source.key === "lobsters") {
      return `https://lobste.rs/s/${source.external_id}`;
    }
    return null;
  };

  return (
    <>
      <div class="feed-controls">
        <div class="control-group">
          <label class="control-label">Source</label>
          <div class="button-group">
            <For each={SOURCE_OPTIONS}>
              {(opt) => (
                <button
                  class={`toggle-btn ${sourceFilter() === opt.value ? "active" : ""}`}
                  onClick={() => handleSourceChange(opt.value)}
                >
                  {opt.label}
                </button>
              )}
            </For>
          </div>
        </div>
        <div class="control-group">
          <label class="control-label">Ranking</label>
          <div class="button-group">
            <For each={RANKING_OPTIONS}>
              {(opt) => (
                <button
                  class={`toggle-btn ${ranking() === opt.value ? "active" : ""}`}
                  onClick={() => handleRankingChange(opt.value)}
                  title={opt.description}
                >
                  {opt.label}
                </button>
              )}
            </For>
          </div>
        </div>
      </div>

      <Show when={newItemsCount() > 0}>
        <div class="new-items-banner">
          <span>{newItemsCount()} new items available</span>
          <button onClick={refresh} class="refresh-btn">
            Refresh
          </button>
        </div>
      </Show>

      <Show when={!loading()} fallback={<p>Loading...</p>}>
        <Show when={!err() || items().length > 0} fallback={<p>{err()}</p>}>
          <ul class="feed-list">
            <For each={items()}>
              {(item) => (
                <li class="feed-entry">
                  <span class="feed-date">
                    {formatRelativeTime(item.submitted_at)}
                  </span>
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
                          {(source, i) => {
                            const url = getSourceDiscussionUrl(source);
                            return (
                              <>
                                {url ? (
                                  <a
                                    href={url}
                                    rel="noopener noreferrer"
                                    class="source-tag source-link"
                                  >
                                    {formatSourceLabel(source)}
                                  </a>
                                ) : (
                                  <span class="source-tag">
                                    {formatSourceLabel(source)}
                                  </span>
                                )}
                                {i() < item.sources.length - 1 && ", "}
                              </>
                            );
                          }}
                        </For>
                      </span>
                    </Show>
                    <span
                      class="similarity-badge"
                      title="Vector similarity to your reading history"
                    >
                      {formatSimilarity(item.similarity_score)}
                    </span>
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
