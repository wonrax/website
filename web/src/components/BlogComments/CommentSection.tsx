import {
  For,
  Show,
  createResource,
  createSignal,
  lazy,
  onCleanup,
  onMount,
  type JSXElement,
  Suspense,
} from "solid-js";
import CommentContext from "./CommentSectionContext";
import config from "@/config";
import { createFetch } from "@/rpc";
import { checkAuthUser } from "@/state";
import { z } from "zod/v4";
import("./CommentSection.scss");

const CommentComponent = lazy(async () => await import("./Comment"));
const CommentSubmission = lazy(async () => ({
  default: (await import("./CommentEditor")).CommentSubmission,
}));

export interface Comment {
  id: number;
  author_name: string;
  content: string;
  parent_id?: number;
  created_at: string;
  children?: Comment[];
  upvote: number;
  depth: number;
  is_blog_author?: boolean;
  is_comment_owner?: boolean;
}

const fetchComments = createFetch(z.custom<Comment[]>());

function countComments(list: Comment[] | undefined): number {
  if (list == null) return 0;
  let n = 0;
  for (const c of list) {
    n += 1;
    n += countComments(c.children);
  }
  return n;
}

export function CommentSection(): JSXElement {
  const slug = window.location.pathname
    .replace(/^\/blog\/?/, "")
    .split("/")
    .filter(Boolean)[0];
  if (slug == null) {
    throw new Error("Why are we rendering comments on a non-blog page?");
  }

  const [pleaseFetch, setPleaseFetch] = createSignal(false);

  const [comments, { mutate, refetch }] = createResource(
    pleaseFetch,
    async () => {
      const res = await fetchComments(
        `${config.API_URL}/blog/${slug}/comments?page_offset=0&page_size=99&sort=best`,
        {
          credentials: "include",
        }
      );

      if (!res.ok) {
        const error = await res.error();
        throw new Error(error.msg);
      }

      return await res.JSON();
    }
  );

  let sectionEl: HTMLElement | undefined;

  onMount(() => {
    if (sectionEl == null) return;

    // 600px rootMargin so the comments fetch fires while the user is still
    // scrolling toward the section, not at the moment it enters view.
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting && !pleaseFetch()) {
            setPleaseFetch(true);
            void CommentComponent.preload();
            void CommentSubmission.preload();
            void checkAuthUser();
            observer.disconnect();
            break;
          }
        }
      },
      { rootMargin: "600px 0px" }
    );

    observer.observe(sectionEl);

    onCleanup(() => observer.disconnect());
  });

  const totalReplies = (): number => countComments(comments());

  return (
    <CommentContext.Provider
      value={{
        refetch: () => {
          void refetch();
        },
        slug,
      }}
    >
      <section
        class="comments-container"
        ref={(el) => (sectionEl = el)}
        aria-labelledby="discussion-heading"
      >
        <h2 id="discussion-heading" class="comments-heading">
          <span class="comments-heading__sigil">§</span>
          <span class="comments-heading__label">discussion</span>
          <Show when={comments.state === "ready" && comments() != null}>
            <span class="comments-heading__count">
              {totalReplies()} {totalReplies() === 1 ? "reply" : "replies"}
            </span>
          </Show>
        </h2>

        <Suspense
          fallback={<p class="ui-meta comments-status">loading discussion…</p>}
        >
          <Show
            when={pleaseFetch()}
            fallback={
              <p class="ui-meta comments-status">scroll to load discussion…</p>
            }
          >
            <Show
              when={
                comments.state !== "unresolved" &&
                comments.state !== "pending" &&
                comments.state !== "refreshing"
              }
              fallback={
                <p class="ui-meta comments-status">loading discussion…</p>
              }
            >
              <div class="comments-compose">
                <CommentSubmission
                  unshift={(c: Comment) => {
                    mutate((comments) => {
                      return [c, ...(comments ?? [])];
                    });
                  }}
                />
              </div>
              <Show when={comments.state === "ready" && comments() != null}>
                <ol class="comments">
                  <For each={comments()}>
                    {(c) => (
                      <CommentComponent
                        comment={c}
                        depth={0}
                        onDelete={() => {
                          mutate((comments) => {
                            return comments?.filter((comment) => {
                              return comment.id !== c.id;
                            });
                          });
                        }}
                      />
                    )}
                  </For>
                </ol>
              </Show>
            </Show>
          </Show>
        </Suspense>
        <Show when={comments.state === "errored"}>
          <p class="comments-error">{`Could not fetch discussions: ${
            (comments.error as Error).message
          }`}</p>
        </Show>
      </section>
    </CommentContext.Provider>
  );
}
