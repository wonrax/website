const config = {
  // env.SITE is configured in astro.config.mts
  API_URL: (import.meta.env.SITE as string | undefined)
    ? (import.meta.env.SITE as string) + "/api"
    : import.meta.env.PROD
      ? ("https://wrx.sh" as const) + "/api"
      : ("http://localhost:3000" as const),
};

export default config;
