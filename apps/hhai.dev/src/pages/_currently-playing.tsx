import config from "@/config";
import { createFetch } from "@/rpc";
import { Suspense, createResource, type JSXElement } from "solid-js";
import { z } from "zod/v4";

const fetchCurrentlyPlaying = createFetch(
  z.object({
    is_playing: z.boolean(),
    item: z.nullish(
      z.object({
        name: z.string(),
        external_urls: z.object({
          spotify: z.string(),
        }),
        artists: z.array(
          z.object({
            name: z.string(),
          })
        ),
      })
    ),
    currently_playing_type: z.optional(z.string()),
  })
);

export default function CurrentlyPlaying(): JSXElement {
  const [currentlyPlaying] = createResource(async () => {
    const res = await fetchCurrentlyPlaying(
      `${config.API_URL}/currently-playing`
    );
    if (!res.ok) {
      const err = await res.error();
      throw Error(err.msg);
    }

    return await res.JSON();
  });

  return (
    <Suspense fallback={null}>
      {currentlyPlaying()?.item != null && (
        <div class="currently-playing">
          <p>Listening to</p>
          <p>
            <span>🟢</span>
            <a
              href={currentlyPlaying()?.item?.external_urls.spotify}
              target="_blank"
            >
              <strong>{currentlyPlaying()?.item?.name}</strong>
            </a>{" "}
            by {currentlyPlaying()?.item?.artists[0].name}
          </p>
        </div>
      )}
    </Suspense>
  );
}
