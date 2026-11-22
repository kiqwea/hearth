import { ConsolePanel } from "../../components/ConsolePanel";

export function ConsoleTab({ id }: { id: string }) {
  return (
    <div className="flex-1 min-h-0 flex flex-col">
      <ConsolePanel id={id} />
    </div>
  );
}
