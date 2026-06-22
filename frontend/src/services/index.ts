import { ApiAskService } from './askService'
import type { AskService } from './askService'

export const askService: AskService = new ApiAskService()
