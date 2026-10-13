import type { Progress } from './types';

export function confirmedPercent(
  progress: Pick<Progress, 'confirmed_at' | 'confirmed_minutes' | 'required_minutes'> | null,
): number | null {
  const minutes = progress?.confirmed_minutes;
  if (
    !progress?.confirmed_at ||
    typeof minutes !== 'number' ||
    !Number.isFinite(minutes) ||
    !Number.isFinite(progress.required_minutes) ||
    progress.required_minutes <= 0
  )
    return null;
  return Math.floor(Math.min(100, Math.max(0, (minutes * 100) / progress.required_minutes)));
}
