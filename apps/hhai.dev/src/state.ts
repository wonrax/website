import { createStore } from "solid-js/store";
import config from "./config";
import { ApiError } from "./rpc";

interface AuthUser {
  id: number;
  name: string;
  email: string;
}

export const [AppState, SetAppState] = createStore<{
  authUser?: AuthUser | null;
}>();

export async function checkAuthUser(): Promise<AuthUser | undefined> {
  const res = await fetch(`${config.API_URL}/is_auth`, {
    credentials: "include",
  });
  if (res.ok) {
    const body: {
      is_auth: boolean;
      id?: number;
      traits?: {
        email: string;
        name: string;
      };
    } = await res.json();

    if (!body.is_auth || body.traits == null || body.id == null) {
      SetAppState("authUser", null);
      return undefined;
    }

    SetAppState("authUser", {
      id: body.id,
      name: body.traits.name,
      email: body.traits.email,
    });

    return {
      id: body.id,
      name: body.traits.name,
      email: body.traits.email,
    };
  } else {
    const error = ApiError.parse(await res.json());
    throw Error("Couldn't check for authentication status: " + error.msg, {
      cause: error.reason,
    });
  }
}
