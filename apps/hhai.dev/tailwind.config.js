/** @type {import('tailwindcss').Config} */
const common = require("ui/tailwind.config");

module.exports = {
  ...common,
  content: [
    "./app/**/*.{js,ts,jsx,tsx,mdx}",
    "./components/**/*.{js,ts,jsx,tsx,mdx}",
  ],
};
