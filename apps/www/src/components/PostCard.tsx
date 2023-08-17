const PostCard = ({
  title,
  description,
  date,
  mainCategory,
  url,
  image,
}: {
  title: string;
  description: string;
  date: Date;
  mainCategory: string;
  url: string;
  image?: string;
}) => {
  return (
    <a
      href={url}
      className="group grid grid-cols-5 gap-6 items-center rounded-xl -my-4"
    >
      <div className="col-span-3 flex flex-col gap-2">
        <p className="text-sm text-teal-400 font-medium">{mainCategory}</p>
        <p className="text-xl font-medium group-hover:underline">{title}</p>
        <p className="text-sm text-gray-500">{description}</p>
        <div className="w-fit text-gray-400 text-xs">
          {date.toLocaleDateString("en-UK", {
            year: "numeric",
            month: "long",
          })}
        </div>
      </div>
      <div className="col-span-2 w-full aspect-video ">
        {image && (
          <img
            src={image}
            alt={title}
            className="transition-opacity opacity-0 group-hover:opacity-100 aspect-video object-cover rounded-lg"
            loading="lazy"
          />
        )}
      </div>
    </a>
  );
};

export default PostCard;
