/// <reference types="vite/client" />

// CSS-only exports (font-icon stylesheets); no JS type declarations shipped.
declare module "@phosphor-icons/web/regular";
declare module "@phosphor-icons/web/fill";

/** "0.<features>.<commits>", injected by vite.config.ts from scripts/version.mjs. */
declare const __APP_VERSION__: string;
