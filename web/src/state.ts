import { createStore } from "solid-js/store";
import config from "./config";
import { createFetch } from "./rpc";
import { z } from "zod/v4";

interface AuthUser {
  id: number;
  name: string;
  email: string;
  siteOwner: boolean;
}

export const [AppState, SetAppState] = createStore<{
  authUser?: AuthUser | null;
}>();

export async function checkAuthUser(): Promise<AuthUser | undefined> {
  const fetchIsAuth = createFetch(
    z.object({
      is_auth: z.boolean(),
      id: z.optional(z.number()),
      traits: z.optional(
        z.object({
          email: z.string(),
          name: z.string(),
        })
      ),
      site_owner: z.optional(z.boolean()),
    })
  );

  const res = await fetchIsAuth(`${config.API_URL}/is_auth`, {
    credentials: "include",
  });
  if (res.ok) {
    const body = await res.JSON();

    if (!body.is_auth || body.traits == null || body.id == null) {
      SetAppState("authUser", null);
      return undefined;
    }

    SetAppState("authUser", {
      id: body.id,
      name: body.traits.name,
      email: body.traits.email,
      siteOwner: body.site_owner ?? false,
    });

    return {
      id: body.id,
      name: body.traits.name,
      email: body.traits.email,
      siteOwner: body.site_owner ?? false,
    };
  } else {
    const error = await res.error();
    throw Error("Couldn't check for authentication status: " + error.msg, {
      cause: error.reason,
    });
  }
}
