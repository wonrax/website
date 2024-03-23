import SheetContext from "@/components/Sheet/SheetContext";
import {
  For,
  createEffect,
  createResource,
  createSignal,
  lazy,
  type JSXElement,
  Suspense,
} from "solid-js";
import CommentContext from "./CommentSectionContext";
import config from "@/config";
import { ApiError } from "@/rpc";
import { checkAuthUser } from "@/state";
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

export function CommentSection(): JSXElement {
  // parse slug from url in format /blog/:slug
  const slug = window.location.pathname.split("/")[2];

  // TODO remove the open_comments query string when the sheet is closed

  // check if query string contains open comments on page load
  // if so, open the comments
  // const { SheetContext: sheetCtx } = SheetContext;
  const params = new URLSearchParams(window.location.search);
  const openComments = params.get("open_comments");
  if (openComments !== null) {
    createEffect((success: boolean) => {
      if (!success) {
        if (SheetContext.SheetContext().initialized) {
          SheetContext.SheetContext().toggle();
          return true;
        }
      }
      return false;
    }, false);
  }

  // hold fetching until the sheet is opened
  const [pleaseFetch, setPleaseFetch] = createSignal(false);

  const [comments, { mutate, refetch }] = createResource(
    pleaseFetch,
    async () => {
      const res = await fetch(
        `${config.API_URL}/blog/${slug}/comments?page_offset=0&page_size=99&sort=best`,
        {
          credentials: "include",
        },
      );

      if (res.status !== 200) {
        const error = ApiError.parse(await res.json());
        throw new Error(error.msg);
      }

      return (await res.json()) as Comment[];
    },
  );

  // listen to sheet context to preload components and check auth user ahead of time
  createEffect(() => {
    const { SheetContext: sheetCtx } = SheetContext;
    if (
      sheetCtx().initialized &&
      (sheetCtx().isSheetTriggerButtonHovered() || sheetCtx().isOpen()) &&
      !pleaseFetch()
    ) {
      setPleaseFetch(true);
      void CommentComponent.preload();
      void CommentSubmission.preload();
      void checkAuthUser();
    }
  });

  return (
    <CommentContext.Provider
      value={{
        refetch: () => {
          void refetch();
        },
        slug,
      }}
    >
      <div class="comments-container">
        <div class="heading">
          <h3
            onClick={() => {
              void refetch();
            }}
            style={{ cursor: "pointer" }}
          >
            Comments
          </h3>
        </div>
        <Suspense fallback={<span class="loader" />}>
          {comments.state === "unresolved" ||
          comments.state === "pending" ||
          comments.state === "refreshing" ? (
            <span class="loader" />
          ) : (
            <>
              <CommentSubmission
                unshift={(c: Comment) => {
                  mutate((comments) => {
                    return [c, ...(comments ?? [])];
                  });
                }}
              />
              {comments.state === "ready" && comments() != null && (
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
              )}
            </>
          )}
        </Suspense>
        {comments.state === "errored" && (
          <p
            style={{ color: "red" }}
          >{`Error fetching comments: ${(comments.error as Error).message}`}</p>
        )}
      </div>
    </CommentContext.Provider>
  );
}
