const config = {
  API_URL: import.meta.env.PROD
    ? ("https://hhai.dev/api" as const)
    : ("http://localhost:3000" as const),
};

export default config;
