import config from "@/config";
import { ApiError } from "@/rpc";
import { Suspense, createResource, type JSXElement } from "solid-js";

export default function CurrentlyPlaying(): JSXElement {
  const [currentlyPlaying] = createResource(async () => {
    const res = await fetch(`${config.API_URL}/currently-playing`);
    if (!res.ok) {
      const err = ApiError.parse(await res.json());
      throw Error(err.msg);
    }

    // TODO verify schema using zod
    return (await res.json()) as {
      is_playing: boolean;
      item?: {
        name: string;
        external_urls: {
          spotify: string;
        };
        artists: Array<{
          name: string;
        }>;
      };
      currently_playing_type?: string;
    };
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
