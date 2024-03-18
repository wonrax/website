import SheetContext from "@/components/Sheet/SheetContextSolid";
import {
  For,
  createEffect,
  createResource,
  createSignal,
  lazy,
  type JSXElement,
} from "solid-js";
import CommentContext from "./CommentSectionContextSolid";
import config from "@/config";
import { ApiError } from "@/rpc";
import("./CommentSection.scss");

const CommentComponent = lazy(async () => await import("./CommentSolid"));
const CommentEditor = lazy(async () => await import("./CommentEditorSolid"));

export interface Comment {
  id: number;
  author_name: string;
  content: string;
  parent_id?: number;
  created_at: string;
  children?: Comment[];
  upvote: number;
  depth: number;
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
  const [doFetch, setDoFetch] = createSignal(false);

  const [comments, { mutate, refetch }] = createResource(doFetch, async () => {
    const res = await fetch(
      `${config.API_URL}/blog/${slug}/comments?page_offset=0&page_size=99&sort=best`,
    );

    if (res.status !== 200) {
      const error = ApiError.parse(await res.json());
      throw new Error(error.msg);
    }

    return (await res.json()) as Comment[];
  });

  // listen to sheet context
  createEffect(() => {
    const { SheetContext: sheetCtx } = SheetContext;
    if (
      sheetCtx().initialized &&
      (sheetCtx().isTriggerHover() || sheetCtx().isOpen())
    ) {
      setDoFetch(true);
      void CommentComponent.preload();
      void CommentEditor.preload();
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
          {(comments.state === "pending" ||
            comments.state === "refreshing") && <span class="loader" />}
        </div>
        {(comments.state === "ready" || comments.state === "refreshing") &&
          comments() != null && (
            <>
              <CommentEditor
                unshift={(c: Comment) => {
                  mutate((comments) => {
                    return [c, ...(comments ?? [])];
                  });
                }}
              />
              <ol class="comments">
                <For each={comments()}>
                  {(c) => <CommentComponent comment={c} depth={0} />}
                </For>
              </ol>
            </>
          )}
        {comments.state === "errored" && (
          <p
            style={{ color: "red" }}
          >{`Error fetching comments: ${(comments.error as Error).message}`}</p>
        )}
      </div>
    </CommentContext.Provider>
  );
}
