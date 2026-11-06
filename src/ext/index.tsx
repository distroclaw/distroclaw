// DistroClaw Community uses none of the extension points in ./types.ts: the app opens straight
// to the workspace, and every optional slot stays empty.
import type { ReactNode } from "react";
import type { Extension } from "./types";

function Gate({ children }: { children: ReactNode }) {
  return <>{children}</>;
}

export const ext: Extension = {
  Gate,
  editionName: "DistroClaw Community",
  areaViews: {},
  toolLabels: {},
  skillPermissions: [],
};
