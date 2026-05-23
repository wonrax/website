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
import { formatRelativeShort } from "@/utils/time";
import "./RecommenderFeedSolid.scss";

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
type FeedFilters = {
  sourceFilter: SourceFilter;
  ranking: RankingPreset;
};

const SOURCE_OPTIONS: { value: SourceFilter; label: string }[] = [
  { value: "all", label: "all" },
  { value: "lobsters", label: "lobsters" },
  { value: "hacker_news", label: "hacker news" },
];

const RANKING_OPTIONS: { value: RankingPreset; label: string }[] = [
  { value: "balanced", label: "balanced" },
  { value: "top_first", label: "top" },
  { value: "newer_first", label: "newer" },
  { value: "similar_first", label: "similar" },
];

const DEFAULT_FILTERS: FeedFilters = {
  sourceFilter: "all",
  ranking: "balanced",
};

function isSourceFilter(value: string | null): value is SourceFilter {
  return SOURCE_OPTIONS.some((option) => option.value === value);
}

function isRankingPreset(value: string | null): value is RankingPreset {
  return RANKING_OPTIONS.some((option) => option.value === value);
}

function readFiltersFromUrl(): FeedFilters {
  if (typeof window === "undefined") {
    return DEFAULT_FILTERS;
  }

  const params = new URLSearchParams(window.location.search);
  const rawSourceFilter = params.get("source");
  const rawRanking = params.get("ranking");

  return {
    sourceFilter: isSourceFilter(rawSourceFilter)
      ? rawSourceFilter
      : DEFAULT_FILTERS.sourceFilter,
    ranking: isRankingPreset(rawRanking) ? rawRanking : DEFAULT_FILTERS.ranking,
  };
}

function writeFiltersToUrl(
  filters: FeedFilters,
  mode: "push" | "replace"
): void {
  if (typeof window === "undefined") {
    return;
  }

  const url = new URL(window.location.href);

  if (filters.sourceFilter === DEFAULT_FILTERS.sourceFilter) {
    url.searchParams.delete("source");
  } else {
    url.searchParams.set("source", filters.sourceFilter);
  }

  if (filters.ranking === DEFAULT_FILTERS.ranking) {
    url.searchParams.delete("ranking");
  } else {
    url.searchParams.set("ranking", filters.ranking);
  }

  const nextUrl = `${url.pathname}${url.search}${url.hash}`;
  const currentUrl = `${window.location.pathname}${window.location.search}${window.location.hash}`;

  if (nextUrl === currentUrl) {
    return;
  }

  if (mode === "replace") {
    window.history.replaceState(null, "", nextUrl);
  } else {
    window.history.pushState(null, "", nextUrl);
  }
}

function getWebsiteUrl(url: string): string {
  try {
    const parsed = new URL(url);
    return parsed.hostname.replace(/^www\./, "");
  } catch {
    return url;
  }
}

function formatSourceKey(key: string): string {
  if (key === "hacker-news") return "hn";
  return key;
}

function getSourceDiscussionUrl(source: SourceInfo): string | null {
  if (!source.external_id) return null;
  if (source.key === "hacker-news") {
    return `https://news.ycombinator.com/item?id=${source.external_id}`;
  }
  if (source.key === "lobsters") {
    return `https://lobste.rs/s/${source.external_id}`;
  }
  return null;
}

function MatchDots(props: { match: number | null }): JSXElement {
  const width = 5;
  const pct = () => (props.match == null ? 0 : Math.round(props.match * 100));
  const filled = () => {
    if (props.match == null) return 0;
    // Any non-zero match shows at least 1 cell so a "low match" still reads as positive.
    return Math.max(1, Math.round(props.match * width));
  };
  return (
    <span class="recommender-feed__match">
      <Show
        when={props.match != null}
        fallback={<span class="recommender-feed__match-na">—</span>}
      >
        <span class="recommender-feed__match-dots">
          <span class="recommender-feed__match-fill">
            {"▰".repeat(filled())}
          </span>
          <span class="recommender-feed__match-empty">
            {"▱".repeat(width - filled())}
          </span>
        </span>
        <span class="recommender-feed__match-pct">{pct()}%</span>
      </Show>
    </span>
  );
}

function FilterRow<T extends string>(props: {
  label: string;
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
}): JSXElement {
  return (
    <div class="recommender-feed__filter-row">
      <span class="recommender-feed__filter-label">{props.label}</span>
      <div class="recommender-feed__filter-options">
        <For each={props.options}>
          {(opt, i) => (
            <>
              {i() > 0 && (
                <span class="recommender-feed__filter-sep" aria-hidden="true">
                  ·
                </span>
              )}
              <button
                type="button"
                class={
                  "ui-button ui-button--xs ui-button--toggle" +
                  (props.value === opt.value ? " is-active" : "")
                }
                onClick={() => props.onChange(opt.value)}
              >
                {opt.label}
              </button>
            </>
          )}
        </For>
      </div>
    </div>
  );
}

export default function RecommenderFeed(): JSXElement {
  const initialFilters = readFiltersFromUrl();
  const [items, setItems] = createSignal<FeedItem[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [err, setErr] = createSignal<string | null>(null);
  const [newItemsCount, setNewItemsCount] = createSignal(0);
  const [offset, setOffset] = createSignal(0);
  const [hasMore, setHasMore] = createSignal(true);
  const [loadingMore, setLoadingMore] = createSignal(false);

  const [sourceFilter, setSourceFilter] = createSignal<SourceFilter>(
    initialFilters.sourceFilter
  );
  const [ranking, setRanking] = createSignal<RankingPreset>(
    initialFilters.ranking
  );

  const LIMIT = 20;

  const fetchFeed = async (
    currentOffset: number,
    append = false,
    filters?: FeedFilters
  ) => {
    const activeFilters = filters ?? {
      sourceFilter: sourceFilter(),
      ranking: ranking(),
    };

    try {
      const params = new URLSearchParams({
        offset: currentOffset.toString(),
        limit: LIMIT.toString(),
        source: activeFilters.sourceFilter,
        ranking: activeFilters.ranking,
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

  const reloadFeedForFilters = async (
    nextFilters: FeedFilters,
    historyMode: "push" | "replace" | "skip" = "push"
  ) => {
    setSourceFilter(nextFilters.sourceFilter);
    setRanking(nextFilters.ranking);
    setOffset(0);
    setLoading(true);

    if (historyMode !== "skip") {
      writeFiltersToUrl(nextFilters, historyMode);
    }

    await fetchFeed(0, false, nextFilters);
    setLoading(false);
  };

  const loadMore = async () => {
    if (loadingMore() || !hasMore() || loading()) return;
    setLoadingMore(true);
    const newOffset = offset() + LIMIT;
    await fetchFeed(newOffset, true);
    setOffset(newOffset);
    setLoadingMore(false);
  };

  let sentinelRef: HTMLDivElement | undefined;

  const refresh = async () => {
    setLoading(true);
    setOffset(0);
    setNewItemsCount(0);
    await fetchFeed(0, false, {
      sourceFilter: sourceFilter(),
      ranking: ranking(),
    });
    setLoading(false);
  };

  onMount(() => {
    writeFiltersToUrl(
      {
        sourceFilter: sourceFilter(),
        ranking: ranking(),
      },
      "replace"
    );

    toast.loading("Fetching recommendations...", {
      id: "recommender-fetch",
      duration: Infinity,
    });

    fetchFeed(0, false, {
      sourceFilter: sourceFilter(),
      ranking: ranking(),
    }).then(() => {
      setLoading(false);
      toast.dismiss("recommender-fetch");
    });

    const handlePopState = () => {
      void reloadFeedForFilters(readFiltersFromUrl(), "skip");
    };

    window.addEventListener("popstate", handlePopState);

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

    // Infinite scroll: once the sentinel under the list scrolls into view (with
    // a 600px head-start), fire loadMore. The existing "load more ->" link stays
    // as a manual fallback in case JS or the observer fails.
    const sentinelObserver = new IntersectionObserver(
      (entries) => {
        if (
          entries[0]?.isIntersecting &&
          hasMore() &&
          !loadingMore() &&
          !loading()
        ) {
          void loadMore();
        }
      },
      { rootMargin: "600px 0px 600px 0px" }
    );
    if (sentinelRef) {
      sentinelObserver.observe(sentinelRef);
    }

    onCleanup(() => {
      window.removeEventListener("popstate", handlePopState);
      eventSource.close();
      sentinelObserver.disconnect();
    });
  });

  return (
    <div class="recommender-feed">
      <div class="recommender-feed__controls">
        <FilterRow
          label="source"
          value={sourceFilter()}
          options={SOURCE_OPTIONS}
          onChange={(v) =>
            reloadFeedForFilters(
              { sourceFilter: v, ranking: ranking() },
              "push"
            )
          }
        />
        <FilterRow
          label="rank"
          value={ranking()}
          options={RANKING_OPTIONS}
          onChange={(v) =>
            reloadFeedForFilters(
              { sourceFilter: sourceFilter(), ranking: v },
              "push"
            )
          }
        />
      </div>

      <Show when={newItemsCount() > 0}>
        <div class="recommender-feed__new-items">
          <span>{newItemsCount()} new items available</span>
          <button
            type="button"
            class="ui-button ui-button--xs"
            onClick={refresh}
          >
            {"refresh ->"}
          </button>
        </div>
      </Show>

      <div class="recommender-feed__thead" aria-hidden="true">
        <span>#</span>
        <span>when</span>
        <span>article</span>
        <span class="recommender-feed__thead-right">match</span>
      </div>

      <Show
        when={!loading()}
        fallback={<p class="recommender-feed__status">loading…</p>}
      >
        <Show
          when={!err() || items().length > 0}
          fallback={<p class="recommender-feed__status">{err()}</p>}
        >
          <Show
            when={items().length > 0}
            fallback={<p class="recommender-feed__status">no items.</p>}
          >
            <ul class="recommender-feed__list">
              <For each={items()}>
                {(item, i) => {
                  const when = formatRelativeShort(
                    item.submitted_at ? new Date(item.submitted_at + "Z") : null
                  );
                  const domain = getWebsiteUrl(item.url);
                  return (
                    <li class="recommender-feed__entry">
                      <span class="recommender-feed__index">
                        {String(i() + 1).padStart(2, "0")}
                      </span>
                      <span class="recommender-feed__when">{when}</span>
                      <span class="ui-meta recommender-feed__sources">
                        <For each={item.sources}>
                          {(source, j) => {
                            const url = getSourceDiscussionUrl(source);
                            const label = formatSourceKey(source.key);
                            const score =
                              source.score != null
                                ? ` · ${Math.round(source.score)}`
                                : "";
                            return (
                              <>
                                {j() > 0 && (
                                  <span class="recommender-feed__sources-sep">
                                    ·
                                  </span>
                                )}
                                <Show
                                  when={url != null}
                                  fallback={
                                    <span>
                                      {label}
                                      {score}
                                    </span>
                                  }
                                >
                                  <a
                                    class="ui-link ui-link--title"
                                    href={url ?? "#"}
                                    target="_blank"
                                    rel="noopener noreferrer"
                                  >
                                    {label}
                                    {score}
                                  </a>
                                </Show>
                              </>
                            );
                          }}
                        </For>
                      </span>
                      <a
                        href={item.url}
                        target="_blank"
                        rel="noopener noreferrer"
                        class="recommender-feed__title"
                      >
                        {item.title}
                      </a>
                      <a
                        href={item.url}
                        target="_blank"
                        rel="noopener noreferrer"
                        class="ui-link ui-link--title recommender-feed__domain"
                      >
                        {domain}{" "}
                        <span class="recommender-feed__domain-arrow">
                          {"->"}
                        </span>
                      </a>
                      <MatchDots match={item.similarity_score} />
                    </li>
                  );
                }}
              </For>
            </ul>
          </Show>
        </Show>
      </Show>

      <div
        ref={sentinelRef}
        class="recommender-feed__sentinel"
        aria-hidden="true"
      />

      <div class="ui-meta recommender-feed__foot">
        <span>
          {items().length} rows{hasMore() ? " · more available" : " · EOF"}
        </span>
        <Show when={hasMore() && !loading()}>
          <button
            type="button"
            class="ui-button ui-button--xs ui-button--muted"
            onClick={loadMore}
            disabled={loadingMore()}
          >
            {loadingMore() ? "loading…" : "load more ->"}
          </button>
        </Show>
      </div>
    </div>
  );
}
