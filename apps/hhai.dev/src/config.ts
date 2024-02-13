const config = {
  API_URL:
    (import.meta.env.PUBLIC_API_URL as string | undefined) ??
    (import.meta.env.PROD
      ? ("https://hhai.dev/api" as const)
      : ("http://localhost:3000" as const)),
};

export default config;
