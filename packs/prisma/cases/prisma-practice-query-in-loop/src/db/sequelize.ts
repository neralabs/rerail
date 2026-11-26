import { DataTypes, Sequelize } from "sequelize";

const sequelize = new Sequelize(process.env.LEGACY_DATABASE_URL!);
const User = sequelize.define("User", { email: DataTypes.STRING });

// A Sequelize model registry under the name `db`: `db.user.update(…)` reads like Prisma but is not.
export const db = { sequelize, user: User };
