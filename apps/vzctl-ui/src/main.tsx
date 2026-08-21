import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import {
  MutationCache,
  QueryCache,
  QueryClient,
  QueryClientProvider,
} from "@tanstack/react-query";
import { RouterProvider, createRouter } from "@tanstack/react-router";
import {
  demoRoute,
  doctorRoute,
  envRoute,
  errorsRoute,
  imagesRoute,
  indexRoute,
  networksRoute,
  projectsRoute,
  rootRoute,
  settingsRoute,
  vmConsoleRoute,
  vmContainerDetailRoute,
  vmContainersRoute,
  vmDetailRoute,
  vmModifyRoute,
  vmMountRoute,
  vmOverviewRoute,
  vmLogsRoute,
  vmReplaceRoute,
  vmShellRoute,
  vmsRoute,
} from "./routes";
import { enableDemoMode, isDemoMode } from "./lib/demo";
import { reportError } from "./store/errorStore";
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
  vmDetailRoute.addChildren([
    vmOverviewRoute,
    vmLogsRoute,
    vmShellRoute,
    vmConsoleRoute,
    vmModifyRoute,
    vmMountRoute,
    vmReplaceRoute,
    vmContainersRoute,
    vmContainerDetailRoute,
  ]),
  projectsRoute,
  networksRoute,
  imagesRoute,
  doctorRoute,
  errorsRoute,
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
  queryCache: new QueryCache({
    onError: (error, query) => {
      reportError(error, { source: "query", queryKey: query.queryKey });
    },
  }),
  mutationCache: new MutationCache({
    onError: (error, _variables, _context, mutation) => {
      reportError(error, {
        source: "mutation",
        mutationKey: mutation.options.mutationKey,
      });
    },
  }),
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
