import { z } from "zod";

export const DiagramNodePositionSchema = z.object({
  x: z.number(),
  y: z.number(),
  width: z.number().optional(),
  height: z.number().optional(),
  zIndex: z.number().optional(),
});

export const DiagramEdgeLayoutSchema = z.object({
  vertices: z
    .array(z.object({ x: z.number(), y: z.number() }))
    .default([]),
});

export const DiagramViewportSchema = z.object({
  sx: z.number().default(1),
  sy: z.number().default(1),
  ox: z.number().default(0),
  oy: z.number().default(0),
});

/** Layout sidecar — never written into hypernetwork.config.yaml. */
export const DiagramStateSchema = z.object({
  schemaVersion: z.literal(1),
  nodes: z.record(z.string(), DiagramNodePositionSchema).default({}),
  edges: z.record(z.string(), DiagramEdgeLayoutSchema).default({}),
  viewport: DiagramViewportSchema.default({
    sx: 1,
    sy: 1,
    ox: 0,
    oy: 0,
  }),
});

export type DiagramNodePosition = z.infer<typeof DiagramNodePositionSchema>;
export type DiagramEdgeLayout = z.infer<typeof DiagramEdgeLayoutSchema>;
export type DiagramViewport = z.infer<typeof DiagramViewportSchema>;
export type DiagramState = z.infer<typeof DiagramStateSchema>;

export function emptyDiagramState(): DiagramState {
  return {
    schemaVersion: 1,
    nodes: {},
    edges: {},
    viewport: { sx: 1, sy: 1, ox: 0, oy: 0 },
  };
}
