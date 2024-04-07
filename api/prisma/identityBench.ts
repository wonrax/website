import { PrismaClient, type Identity } from "@prisma/client";
import { faker } from "@faker-js/faker";

const prisma = new PrismaClient();

async function main() {
  const credentialType = await prisma.identityCredentialType.upsert({
    create: {
      name: "oauth",
      created_at: new Date(),
    },
    where: {
      name: "oauth",
    },
    update: {
      name: "oauth",
    },
  });

  const identity = await prisma.identity.create({
    data: {
      created_at: new Date(),
      updated_at: new Date(),
    },
  });

  const date = new Date();

  for (let i = 0; i < 2500000; i++) {
    await Promise.all(
      Array(36)
        .fill(0)
        .map(() =>
          prisma.identityCredential.create({
            data: {
              created_at: date,
              updated_at: date,
              credential: {
                oidc_provider: "github",
                provider: "github",
                user_id: faker.number.int(),
              },
              identity_id: identity.id,
              credential_type_id: credentialType.id,
            },
          }),
        ),
    );
  }
}

main()
  .then(async () => {
    await prisma.$disconnect();
  })
  .catch(async (e) => {
    console.error(e);
    await prisma.$disconnect();
    process.exit(1);
  });
