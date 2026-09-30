import { mount } from "svelte";
import "@fontsource/source-sans-3/400.css";
import "@fontsource/source-sans-3/500.css";
import "@fontsource/source-sans-3/600.css";
import "@fontsource/source-sans-3/700.css";
import "@phosphor-icons/web/regular";
import "@phosphor-icons/web/fill";
import "./lib/app.css";
import App from "./App.svelte";
const app = mount(App, { target: document.getElementById("app")! });

// The window was already shown by index.html with the preloader; drop it now that the UI is mounted.
document.getElementById("boot")?.remove();

export default app;
