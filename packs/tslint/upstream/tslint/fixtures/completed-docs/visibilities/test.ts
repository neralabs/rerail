export enum BadExportedEnum { }

/**
 * ...
 */
export enum ExportedEnum {
    BadExportedMember
 }

/**
 * ...
 */
export enum GoodExportedEnum { }

enum BadInternalEnum {
    InternalMember
 }

/**
 * ...
 */
enum GoodInternalEnum {
    /**
     * ...
     */
    GoodInternalMember
}

export interface BadExportedInterface { }

export interface GoodExportedInterface { }

interface BadInternalInterface { }

/**
 * ...
 */
interface GoodInternalInterface { }

const internalVariable = 1;

export const exportedVariable = 2;
