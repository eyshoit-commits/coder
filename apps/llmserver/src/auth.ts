import jwt from "jsonwebtoken";
import { ServerConfig } from "./config";

export interface AdminClaims {
  sub: string;
  role: string;
  exp: number;
}

export function verifyAdminToken(config: ServerConfig, token: string): AdminClaims {
  try {
    const payload = jwt.verify(token, config.adminJwtSecret);
    const claims = payload as AdminClaims;
    if (claims.role !== "admin") {
      throw new Error("Token does not grant admin privileges");
    }
    return claims;
  } catch (error) {
    throw new Error(`Invalid admin token: ${String(error)}`);
  }
}
