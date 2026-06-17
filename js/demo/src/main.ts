import { mount } from "svelte";
import App from "./App.svelte";
import "@xterm/xterm/css/xterm.css";

const app = mount(App, { target: document.getElementById("app")! });

export default app;
