import { Graph } from "@antv/x6";

export const SHAPE_VM = "vzctl-vm";
export const SHAPE_NETWORK = "vzctl-network";
export const SHAPE_GATEWAY = "vzctl-gateway";
export const SHAPE_EDGE = "vzctl-edge";

const portCircle = {
  r: 5,
  magnet: true,
  strokeWidth: 1.5,
};

export function registerShapes(): void {
  // Overwrite registrations so port groups stay in sync with this module.

  Graph.registerNode(
    SHAPE_VM,
    {
      inherit: "rect",
      width: 200,
      height: 100,
      attrs: {
        body: {
          strokeWidth: 1.5,
          stroke: "#1c2b27",
          fill: "#fffaf0",
          rx: 8,
          ry: 8,
        },
        title: {
          text: "VM",
          refX: 12,
          refY: 18,
          fill: "#1c1915",
          fontSize: 13,
          fontWeight: 600,
          textAnchor: "start",
          textVerticalAnchor: "middle",
        },
        subtitle: {
          text: "",
          refX: 12,
          refY: 40,
          fill: "#5b564c",
          fontSize: 11,
          textAnchor: "start",
          textVerticalAnchor: "middle",
        },
        nics: {
          text: "",
          refX: 12,
          refY: 62,
          fill: "#0f6a5a",
          fontSize: 11,
          textAnchor: "start",
          textVerticalAnchor: "top",
        },
      },
      markup: [
        { tagName: "rect", selector: "body" },
        { tagName: "text", selector: "title" },
        { tagName: "text", selector: "subtitle" },
        { tagName: "text", selector: "nics" },
      ],
      ports: {
        groups: {
          /** Connected secondary NICs + free "new" port. */
          nic: {
            position: {
              name: "right",
              args: { strict: true },
            },
            attrs: {
              circle: {
                ...portCircle,
                stroke: "#0f6a5a",
                fill: "#fffaf0",
              },
            },
          },
        },
        items: [],
      },
    },
    true,
  );

  Graph.registerNode(
    SHAPE_NETWORK,
    {
      inherit: "rect",
      width: 360,
      height: 280,
      zIndex: 1,
      attrs: {
        body: {
          strokeWidth: 1.5,
          stroke: "#0f6a5a",
          fill: "#eef6f3",
          rx: 12,
          ry: 12,
        },
        header: {
          refWidth: "100%",
          height: 40,
          strokeWidth: 0,
          fill: "#d8ebe4",
          rx: 12,
          ry: 12,
        },
        headerClip: {
          refWidth: "100%",
          y: 20,
          height: 20,
          fill: "#d8ebe4",
          strokeWidth: 0,
        },
        title: {
          text: "Network",
          refX: 14,
          refY: 14,
          fill: "#1c1915",
          fontSize: 13,
          fontWeight: 600,
          textAnchor: "start",
          textVerticalAnchor: "middle",
        },
        subtitle: {
          text: "",
          refX: 14,
          refY: 28,
          fill: "#5b564c",
          fontSize: 10,
          textAnchor: "start",
          textVerticalAnchor: "middle",
        },
        meta: {
          text: "",
          refX: "98%",
          refY: 20,
          fill: "#5b564c",
          fontSize: 10,
          textAnchor: "end",
          textVerticalAnchor: "middle",
        },
      },
      markup: [
        { tagName: "rect", selector: "body" },
        { tagName: "rect", selector: "header" },
        { tagName: "rect", selector: "headerClip" },
        { tagName: "text", selector: "title" },
        { tagName: "text", selector: "subtitle" },
        { tagName: "text", selector: "meta" },
      ],
      ports: {
        groups: {
          uplink: {
            position: { name: "top", args: { strict: true } },
            attrs: {
              circle: {
                ...portCircle,
                stroke: "#5b564c",
                fill: "#f3efe6",
              },
            },
          },
          /** NIC attachments (multi-homing) */
          attach: {
            position: { name: "bottom", args: { strict: true } },
            attrs: {
              circle: {
                ...portCircle,
                stroke: "#0f6a5a",
                fill: "#fffaf0",
                r: 6,
              },
            },
          },
          routeLeft: {
            position: { name: "left", args: { strict: true } },
            attrs: {
              circle: {
                ...portCircle,
                stroke: "#9b2c2c",
                fill: "#faf0f0",
              },
            },
          },
          routeRight: {
            position: { name: "right", args: { strict: true } },
            attrs: {
              circle: {
                ...portCircle,
                stroke: "#9b2c2c",
                fill: "#faf0f0",
              },
            },
          },
        },
        items: [],
      },
    },
    true,
  );

  Graph.registerNode(
    SHAPE_GATEWAY,
    {
      inherit: "path",
      width: 100,
      height: 68,
      // Internet/cloud silhouette — scaled via refD to node bbox
      path: "M 25 55 C 8 55 2 42 8 32 C 0 28 -2 16 10 14 C 12 4 24 -2 36 4 C 44 -4 62 -2 68 10 C 82 6 96 16 92 30 C 102 34 100 50 88 54 C 82 62 40 64 25 55 Z",
      attrs: {
        body: {
          strokeWidth: 1.5,
          stroke: "#5b564c",
          fill: "#e8f0f6",
        },
        label: {
          text: "Internet",
          fill: "#3d4a52",
          fontSize: 11,
          fontWeight: 600,
          refX: "50%",
          refY: "55%",
          textAnchor: "middle",
          textVerticalAnchor: "middle",
        },
      },
      ports: {
        groups: {
          down: {
            position: { name: "bottom", args: { strict: true } },
            attrs: {
              circle: {
                ...portCircle,
                stroke: "#5b564c",
                fill: "#fffaf0",
              },
            },
          },
        },
        items: [{ id: "down", group: "down" }],
      },
    },
    true,
  );

  Graph.registerEdge(
    SHAPE_EDGE,
    {
      inherit: "edge",
      attrs: {
        line: {
          stroke: "#5b564c",
          strokeWidth: 1.5,
          targetMarker: { name: "classic", size: 7 },
        },
      },
      router: {
        name: "manhattan",
        args: { padding: 16, startDirections: ["top"], endDirections: ["bottom"] },
      },
      connector: { name: "rounded", args: { radius: 8 } },
    },
    true,
  );
}
