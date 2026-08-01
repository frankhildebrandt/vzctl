import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider, createRouter } from "@tanstack/react-router";
import {
  demoRoute,
  doctorRoute,
  envRoute,
  imagesRoute,
  indexRoute,
  networksRoute,
  projectsRoute,
  rootRoute,
  settingsRoute,
  vmContainerDetailRoute,
  vmContainersRoute,
  vmDetailRoute,
  vmsRoute,
} from "./routes";
import { enableDemoMode, isDemoMode } from "./lib/demo";
import { useSettingsStore } from "./store/settingsStore";
import "./styles.css";

// Ensure theme is applied before first paint of React tree.
void useSettingsStore.getState();

if (isDemoMode()) {
  enableDemoMode();
}

const routeTree = rootRoute.addChildren([
  indexRoute,
  vmsRoute,
  vmContainerDetailRoute,
  vmContainersRoute,
  vmDetailRoute,
  projectsRoute,
  networksRoute,
  imagesRoute,
  doctorRoute,
  settingsRoute,
  demoRoute,
  envRoute,
]);

const router = createRouter({
  routeTree,
  defaultPreload: "intent",
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: false,
      refetchOnWindowFocus: false,
    },
    mutations: {
      retry: false,
    },
  },
});

const el = document.getElementById("root");
if (!el) {
  throw new Error("root element missing");
}

createRoot(el).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  </StrictMode>,
);
