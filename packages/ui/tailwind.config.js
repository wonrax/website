/** @type {import('tailwindcss').Config} */
const defaultTheme = require("tailwindcss/defaultTheme");

module.exports = {
  content: [
    "../../packages/**/*.{js,ts,jsx,tsx}",
    "./**/*.tsx", // TODO don't put it here, set it per project
    "./**/*.astro", // Same here
  ],
  theme: {
    extend: {
      fontFamily: {
        sans: ["Inter", ...defaultTheme.fontFamily.sans],
      },
      typography: {
        DEFAULT: {
          // remove backticks around inline code
          css: {
            "code::before": {
              content: '""',
            },
            "code::after": {
              content: '""',
            },
          },
        },
      },
    },
  },
  plugins: [require("@tailwindcss/typography")], // TODO refactor or move this to elsewhere
};
