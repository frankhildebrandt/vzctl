import { Fragment, type HTMLAttributes, type ReactNode } from "react";
import { cx } from "./cx";

export type DescriptionItem = {
  label: ReactNode;
  value: ReactNode;
  key?: string;
};

export type DescriptionListProps = HTMLAttributes<HTMLDListElement> & {
  items: DescriptionItem[];
  /** Wrap each pair in a row div (for multi-line / nested values). */
  stacked?: boolean;
};

export function DescriptionList({
  items,
  stacked = false,
  className,
  ...props
}: DescriptionListProps) {
  return (
    <dl className={cx("kv", className)} {...props}>
      {items.map((item, index) => {
        const key = item.key ?? String(index);
        if (stacked) {
          return (
            <div key={key} className="kv-row">
              <dt>{item.label}</dt>
              <dd>{item.value}</dd>
            </div>
          );
        }
        return (
          <Fragment key={key}>
            <dt>{item.label}</dt>
            <dd>{item.value}</dd>
          </Fragment>
        );
      })}
    </dl>
  );
}
