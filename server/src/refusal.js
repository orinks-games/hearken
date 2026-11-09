// A handler's or the service's own refusal of a request: becomes
// 422 { error: code, message } for the game to speak.

export class Refusal extends Error {
  constructor(code, message) {
    super(message ?? code);
    this.code = code;
  }
}

export function refuse(code, message) {
  return new Refusal(code, message);
}
