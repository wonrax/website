import { createResource, For, Show, type JSXElement } from "solid-js";
import config from "@/config";
import { parseFeed, type RSSItem } from "../shared/parseRssFeed";
import { formatRelativeShort } from "@/utils/time";

async function fetchRecentBookmarks(): Promise<RSSItem[]> {
  const resp = await fetch(`${config.API_URL}/great-reads-feed`);
  if (!resp.ok) {
    throw new Error(`bookmarks api error: ${resp.status}`);
  }
  const xml = await resp.text();
  return parseFeed(xml).slice(0, 3);
}

export default function HomeRecentBookmarks(): JSXElement {
  const [items] = createResource(fetchRecentBookmarks);

  return (
    <ul class="home__recent-list">
      <Show
        when={!items.loading}
        fallback={
          <For each={[0, 1, 2]}>
            {() => (
              <li class="home__recent-row home__recent-row--skeleton">
                <span class="ui-meta home__recent-date" aria-hidden="true">
                  ──
                </span>
                <span class="home__recent-skeleton-title" aria-hidden="true" />
              </li>
            )}
          </For>
        }
      >
        <Show
          when={!items.error && (items()?.length ?? 0) > 0}
          fallback={
            <li class="home__recent-row home__recent-row--empty">
              <span class="home__recent-empty">
                {items.error ? "couldn't load bookmarks." : "no bookmarks yet."}
              </span>
            </li>
          }
        >
          <For each={items()}>
            {(item) => (
              <li class="home__recent-row">
                <span class="ui-meta home__recent-date">
                  {formatRelativeShort(
                    item.pubDate ? new Date(item.pubDate) : null
                  )}
                </span>
                <a
                  class="ui-link ui-link--title"
                  href={item.link}
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  {item.title}
                </a>
              </li>
            )}
          </For>
        </Show>
      </Show>
    </ul>
  );
}
