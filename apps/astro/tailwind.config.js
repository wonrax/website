/** @type {import('tailwindcss').Config} */
const common = require("ui/tailwind.config");

module.exports = {
  ...common,
  content: ["./src/**/*.{js,ts,jsx,tsx,mdx,astro}"],
  corePlugins: {
    preflight: false,
  },
};
