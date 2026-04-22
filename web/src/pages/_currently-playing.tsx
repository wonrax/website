import config from "@/config";
import { createFetch } from "@/rpc";
import { Suspense, createResource, type JSXElement } from "solid-js";
import { z } from "zod/v4";
import "./_currently-playing.scss";

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
    currently_playing_type: z.nullish(z.string()),
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
  const item = () => currentlyPlaying()?.item;
  const primaryArtistName = () => item()?.artists[0]?.name;

  return (
    <Suspense fallback={null}>
      {item() != null && (
        <div class="currently-playing">
          <p>Listening to</p>
          <p>
            <span class="currently-playing__status" aria-hidden="true" />
            <a
              href={item()?.external_urls.spotify}
              target="_blank"
              rel="noopener noreferrer"
            >
              <strong>{item()?.name}</strong>
            </a>{" "}
            {primaryArtistName() != null ? <>by {primaryArtistName()}</> : null}
          </p>
        </div>
      )}
    </Suspense>
  );
}
