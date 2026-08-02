import type { HTMLAttributes, ReactNode, TableHTMLAttributes } from "react";
import { Card } from "./Card";
import { cx } from "./cx";

export type TableCardProps = HTMLAttributes<HTMLDivElement> & {
  children: ReactNode;
};

/** Card wrapper for data tables (zero padding, scrollable). */
export function TableCard({ className, children, ...props }: TableCardProps) {
  return (
    <Card padding="none" className={className} {...props}>
      {children}
    </Card>
  );
}

export type DataTableProps = TableHTMLAttributes<HTMLTableElement>;

/** Standard data table using `.vm-table` design tokens. */
export function DataTable({ className, children, ...props }: DataTableProps) {
  return (
    <table className={cx("vm-table", className)} {...props}>
      {children}
    </table>
  );
}
