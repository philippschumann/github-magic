// Settings for `npx web-ext run|lint|sign`. Signing is unlisted: Mozilla signs it, it is not published on AMO.
export default {
  ignoreFiles: ["native-host", "native-host/**", "desktop", "desktop/**", "docs", "docs/**", "README.md", "web-ext-config.mjs"],
  sign: { channel: "unlisted" },
};
