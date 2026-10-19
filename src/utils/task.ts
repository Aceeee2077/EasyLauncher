export type Run = <T>(
  key: string,
  fn: () => Promise<T>,
) => Promise<T | undefined>;
export interface TaskProps {
  run: Run;
  busy: Set<string>;
}
