import eslint from "@eslint/js";
import svelte from "eslint-plugin-svelte";
import globals from "globals";
import tseslint from "typescript-eslint";

export default tseslint.config(
  {
    ignores: [
      "**/node_modules/**",
      "apps/web/.svelte-kit/**",
      "apps/web/build/**",
      "coverage/**",
      "var/**"
    ]
  },
  eslint.configs.recommended,
  ...tseslint.configs.recommended,
  ...svelte.configs["flat/recommended"],
  {
    languageOptions: {
      globals: {
        ...globals.browser,
        ...globals.node,
        Bun: "readonly"
      }
    },
    rules: {
      "@typescript-eslint/no-explicit-any": "off",
      "svelte/no-navigation-without-resolve": "off"
    }
  },
  {
    files: ["**/*.svelte"],
    languageOptions: {
      parserOptions: {
        parser: tseslint.parser
      }
    }
  }
);
