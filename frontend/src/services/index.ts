import { StubAskService } from './askService'
import type { AskService } from './askService'
// Plan 3 swaps this line for: export const askService: AskService = new ApiAskService()
export const askService: AskService = new StubAskService()
