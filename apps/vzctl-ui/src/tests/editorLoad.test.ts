import { expect, it } from "vitest";
import { scaffoldEnvironment } from "@/application/services/scaffold";
import { emptyDiagramState } from "@/domain/diagram/types";
import { useEditorStore } from "@/store/editorStore";

it("preserves saved positions on load and fills missing nodes without marking dirty", () => {
  const env = scaffoldEnvironment({ name: "saved" });
  const diagram = emptyDiagramState();
  diagram.nodes["net:lan"] = { x: 1200, y: 800, width: 600, height: 500 };
  useEditorStore.getState().load("/saved", env, diagram);
  const state = useEditorStore.getState();
  expect(state.diagram.nodes["net:lan"]).toEqual(diagram.nodes["net:lan"]);
  expect(state.diagram.nodes["igw:lan"]).toBeDefined();
  expect(state.ui.dirty).toBe(false);
});
