const config = {
  API_URL:
    ((import.meta.env.PUBLIC_SITE_URL as string | undefined) ??
      (import.meta.env.PROD
        ? ("https://wrx.sh" as const)
        : ("http://localhost:3000" as const))) + "/api",
};

export default config;
