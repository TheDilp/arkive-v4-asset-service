use aws_sdk_s3::{ types::ObjectIdentifier, Client };

use crate::enums::AppResponse;

pub async fn recursive_delete(
    client: &Client,
    bucket: &str,
    prefix: &str
) -> Result<(), AppResponse> {
    let mut continuation_token = None;

    loop {
        let list_resp = client
            .list_objects_v2()
            .bucket(bucket)
            .prefix(prefix)
            .set_continuation_token(continuation_token)
            .send().await;

        if list_resp.is_err() {
            break;
        }

        let list_resp = list_resp.unwrap();

        if let Some(objects) = list_resp.contents {
            let keys_to_delete: Vec<ObjectIdentifier> = objects
                .iter()
                .map(|obj|
                    ObjectIdentifier::builder().key(obj.key.as_ref().unwrap()).build().unwrap()
                )
                .collect();

            if !keys_to_delete.is_empty() {
                let _ = client
                    .delete_objects()
                    .bucket(bucket)
                    .delete(
                        aws_sdk_s3::types::Delete
                            ::builder()
                            .set_objects(Some(keys_to_delete))
                            .build()
                            .unwrap()
                    )
                    .send().await
                    .unwrap();

                println!("Deleted {} objects", objects.len());
            } else {
                println!("No objects found with the prefix: {}", prefix);
            }
        }

        if let Some(is_truncated) = list_resp.is_truncated {
            if is_truncated == true {
                continuation_token = list_resp.next_continuation_token;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    Ok(())
}
