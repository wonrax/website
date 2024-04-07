import { AppState, checkAuthUser } from "@/state";
import { Show, createEffect, type JSXElement } from "solid-js";

export default function AccountInfo(): JSXElement {
  createEffect(() => {
    try {
      void checkAuthUser();
    } catch (e) {}
  });

  return (
    <>
      <a href="/">Homepage</a>
      <Show when={AppState.authUser == null}>Loading user...</Show>
      <Show when={AppState.authUser != null}>
        <div>
          <h3>{AppState.authUser?.name}</h3>
          <p>{AppState.authUser?.email}</p>
        </div>
      </Show>
    </>
  );
}
