const { withContentlayer } = require("next-contentlayer");
const withOptimisedImages = require("@hashicorp/next-optimized-images");

const nextConfig = {
  output: "export",
  // Optional: Add a trailing slash to all paths `/about` -> `/about/`
  // trailingSlash: true,
  // Optional: Change the output directory `out` -> `dist`
  // distDir: 'dist',
  images: {
    disableStaticImages: true,
    unoptimized: true,
  },
  // webpack: (config, options) => {
  //   console.log(config);
  //   config.module.rules.push({
  //     test: /\.(jpe?g|png)$/i,
  //     use: [
  //       {
  //         loader: "responsive-loader",
  //         options: {
  //           adapter: require("responsive-loader/sharp"),
  //           publicPath: "/_next/static/media",
  //           outputPath: "../static/media",
  //           name: "[hash]-[width].[ext]",
  //           placeholder: true,
  //         },
  //       },
  //     ],
  //   });

  //   return config;
  // },
};

module.exports = withContentlayer(
  withOptimisedImages({
    ...nextConfig,
    // responsive: {
    //   adapter: require("responsive-loader/sharp"),
    //   sizes: [320, 640, 960, 1200, 1800, 2400],
    //   placeholder: true,
    //   placeholderSize: 20,
    // },
  })
);
