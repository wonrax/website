// Provide proxy support for undici (astro's dependency) fetch requests.
// Improvised version of
// https://github.com/nodejs/undici/issues/1650#issuecomment-1346049805

import { URL } from "node:url";
import {
  getGlobalDispatcher,
  setGlobalDispatcher,
  Dispatcher,
  ProxyAgent,
} from "undici";

const getProxyAgent = (proto: "http" | "https") => {
  let agent: ProxyAgent | undefined;
  if (proto === "http") {
    agent = process.env["https_proxy"]
      ? new ProxyAgent(process.env["https_proxy"])
      : undefined;
  }
  if (!agent) {
    agent = process.env["http_proxy"]
      ? new ProxyAgent(process.env["http_proxy"])
      : undefined;
  }
  return agent;
};

const noProxyRules = (process.env["no_proxy"] ?? "")
  .split(",")
  .map((rule) => rule.trim());

const defaultDispatcher = getGlobalDispatcher();

setGlobalDispatcher(
  new (class extends Dispatcher {
    dispatch(options: any, handler: any) {
      if (options.origin) {
        const { host, protocol } =
          typeof options.origin === "string"
            ? new URL(options.origin)
            : options.origin;
        const hostWithoutPort = host.split(":")[0];
        if (
          !noProxyRules.some((rule) =>
            rule.startsWith(".")
              ? hostWithoutPort.endsWith(rule)
              : hostWithoutPort === rule
          )
        ) {
          const proxyAgent = getProxyAgent(protocol);
          if (proxyAgent) {
            proxyAgent.dispatch(options, handler);
          }
        }
      }
      return defaultDispatcher.dispatch(options, handler);
    }
  })()
);
