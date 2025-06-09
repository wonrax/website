const config = {
  API_URL:
    (import.meta.env.PUBLIC_SITE_URL as string | undefined) ??
    (import.meta.env.PROD
      ? ("https://wrx.sh" as const) + "/api"
      : ("http://localhost:3000" as const)),
};

export default config;
