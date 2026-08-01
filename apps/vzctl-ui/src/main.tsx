import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider, createRouter } from "@tanstack/react-router";
import {
  doctorRoute,
  envRoute,
  imagesRoute,
  indexRoute,
  networksRoute,
  projectsRoute,
  rootRoute,
  vmDetailRoute,
  vmsRoute,
} from "./routes";
import "./styles.css";

const routeTree = rootRoute.addChildren([
  indexRoute,
  vmsRoute,
  vmDetailRoute,
  projectsRoute,
  networksRoute,
  imagesRoute,
  doctorRoute,
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
