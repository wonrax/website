import config from "@/config";
import { AppState, checkAuthUser } from "@/state";
import { Show, createEffect, createResource, type JSXElement } from "solid-js";

export default function AccountInfo(): JSXElement {
  createEffect(() => {
    checkAuthUser()
      .then((user) => {
        if (user == null) {
          window.location.href = "/login";
        }
      })
      .catch(() => {});
  });

  const [connectedApps] = createResource(async () => {
    // TODO verify schema using zod
    const res = await fetch(`${config.API_URL}/link/apps`, {
      credentials: "include",
    });
    return (await res.json()) as {
      github?: {
        user_id: number;
        added_on: string;
      };
      spotify?: {
        display_name: string;
        added_on: string;
      };
    };
  });

  return (
    <>
      <a href="/">Homepage</a>
      <Show when={AppState.authUser == null}>
        <p>Loading user...</p>
      </Show>

      <Show when={AppState.authUser != null}>
        <div class="account">
          <p>Your account</p>
          <h3>{AppState.authUser?.name}</h3>
          <p>{AppState.authUser?.email}</p>
        </div>
      </Show>

      <Show when={connectedApps.state === "ready"}>
        <div class="connections">
          <h3>Integrations</h3>
          <div class="connection">
            <svg
              xmlns="http://www.w3.org/2000/svg"
              height="32px"
              width="32px"
              version="1.1"
              viewBox="0 0 168 168"
            >
              <path
                fill="#1ED760"
                d="m83.996 0.277c-46.249 0-83.743 37.493-83.743 83.742 0 46.251 37.494 83.741 83.743 83.741 46.254 0 83.744-37.49 83.744-83.741 0-46.246-37.49-83.738-83.745-83.738l0.001-0.004zm38.404 120.78c-1.5 2.46-4.72 3.24-7.18 1.73-19.662-12.01-44.414-14.73-73.564-8.07-2.809 0.64-5.609-1.12-6.249-3.93-0.643-2.81 1.11-5.61 3.926-6.25 31.9-7.291 59.263-4.15 81.337 9.34 2.46 1.51 3.24 4.72 1.73 7.18zm10.25-22.805c-1.89 3.075-5.91 4.045-8.98 2.155-22.51-13.839-56.823-17.846-83.448-9.764-3.453 1.043-7.1-0.903-8.148-4.35-1.04-3.453 0.907-7.093 4.354-8.143 30.413-9.228 68.222-4.758 94.072 11.127 3.07 1.89 4.04 5.91 2.15 8.976v-0.001zm0.88-23.744c-26.99-16.031-71.52-17.505-97.289-9.684-4.138 1.255-8.514-1.081-9.768-5.219-1.254-4.14 1.08-8.513 5.221-9.771 29.581-8.98 78.756-7.245 109.83 11.202 3.73 2.209 4.95 7.016 2.74 10.733-2.2 3.722-7.02 4.949-10.73 2.739z"
              />
            </svg>
            <div>
              <h4>Spotify</h4>
              {connectedApps()?.spotify == null ? (
                <p>Show what you're listening to</p>
              ) : (
                <>
                  <p>
                    <span>{connectedApps()?.spotify?.display_name}</span>
                    <span>•</span>
                    <span>
                      Added on{" "}
                      {new Date(
                        connectedApps()?.spotify?.added_on ?? "",
                      ).toLocaleDateString()}
                    </span>
                  </p>
                </>
              )}
            </div>
            {connectedApps()?.spotify == null && (
              <button
                onClick={() =>
                  (window.location.href = `${config.API_URL}/link/spotify`)
                }
              >
                Connect
              </button>
            )}
          </div>
          <div class="connection">
            <svg
              height="32"
              aria-hidden="true"
              viewBox="0 0 16 16"
              version="1.1"
              width="32"
              data-view-component="true"
              fill="currentcolor"
            >
              <path d="M8 0c4.42 0 8 3.58 8 8a8.013 8.013 0 0 1-5.45 7.59c-.4.08-.55-.17-.55-.38 0-.27.01-1.13.01-2.2 0-.75-.25-1.23-.54-1.48 1.78-.2 3.65-.88 3.65-3.95 0-.88-.31-1.59-.82-2.15.08-.2.36-1.02-.08-2.12 0 0-.67-.22-2.2.82-.64-.18-1.32-.27-2-.27-.68 0-1.36.09-2 .27-1.53-1.03-2.2-.82-2.2-.82-.44 1.1-.16 1.92-.08 2.12-.51.56-.82 1.28-.82 2.15 0 3.06 1.86 3.75 3.64 3.95-.23.2-.44.55-.51 1.07-.46.21-1.61.55-2.33-.66-.15-.24-.6-.83-1.23-.82-.67.01-.27.38.01.53.34.19.73.9.82 1.13.16.45.68 1.31 2.69.94 0 .67.01 1.3.01 1.49 0 .21-.15.45-.55.38A7.995 7.995 0 0 1 0 8c0-4.42 3.58-8 8-8Z" />
            </svg>
            <div>
              <h4>GitHub</h4>
              <p>
                <span>User ID {connectedApps()?.github?.user_id}</span>
                <span>•</span>
                <span>
                  Added on{" "}
                  {new Date(
                    connectedApps()?.github?.added_on ?? "",
                  ).toLocaleDateString()}
                </span>
              </p>
            </div>
          </div>
        </div>
      </Show>
    </>
  );
}
